//! Embedded repository picker: deterministic state, terminal effects at the edge.

use std::cmp::Reverse;
use std::env;
use std::fs::{File, OpenOptions};
use std::io::{self, Write as _};
use std::os::fd::{AsFd as _, OwnedFd};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossterm::cursor::Show;
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32String};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, ListState, Padding, Paragraph};
use ratatui::{Frame, Terminal, TerminalOptions, Viewport};

use crate::Result;
use crate::discovery::Repository;
use crate::info::{self, Info};
use crate::visits::Visits;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc;

const MAX_QUERY: usize = 1024;

struct Entry {
    path: PathBuf,
    name: String,
    display: String,
    name_text: Utf32String,
    path_text: Utf32String,
    /// Ranking boost from frecency: how often and how recently you went there.
    boost: u32,
    /// Your latest Git activity, which orders checkouts you have not visited yet.
    activity: Option<u64>,
}

use crate::ui::visible;

/// Rows used by the inline picker: a prompt, the results and a key hint.
const HEIGHT: u16 = 12;

struct Picker {
    entries: Vec<Entry>,
    matches: Vec<usize>,
    query: Vec<char>,
    cursor: usize,
    selection: ListState,
    matcher: Matcher,
    pattern: Pattern,
    monochrome: bool,
    /// Details for the preview pane, filled in by a background worker.
    previews: HashMap<PathBuf, Info>,
}

/// The preview pane appears beside the results from this terminal width.
const PREVIEW_WIDTH: u16 = 100;

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Continue,
    Select(PathBuf),
    Cancel,
    Suspend,
}

impl Picker {
    fn new(
        repositories: &[Repository],
        roots: &[PathBuf],
        visits: &HashMap<PathBuf, Visits>,
        query: &str,
        monochrome: bool,
    ) -> Self {
        let entries = repositories
            .iter()
            .map(|repository| {
                let name = visible(
                    &repository
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                );
                let display = crate::ui::repository_name(&repository.path, roots);
                let frecency = visits
                    .get(&repository.path)
                    .map_or(0, |visits| visits.frecency);
                Entry {
                    path: repository.path.clone(),
                    name_text: Utf32String::from(name.as_str()),
                    path_text: Utf32String::from(display.as_str()),
                    name,
                    display,
                    boost: u32::try_from(frecency.isqrt().saturating_mul(4)).unwrap_or(u32::MAX),
                    activity: crate::visits::activity(&repository.path),
                }
            })
            .collect();
        let query: Vec<_> = query
            .chars()
            .filter(|character| !character.is_control())
            .take(MAX_QUERY)
            .collect();
        let mut picker = Self {
            entries,
            matches: Vec::new(),
            cursor: query.len(),
            query,
            selection: ListState::default(),
            matcher: Matcher::new(Config::DEFAULT),
            pattern: Pattern::default(),
            monochrome,
            previews: HashMap::new(),
        };
        picker.filter();
        picker
    }

    fn filter(&mut self) {
        self.pattern = Pattern::new(
            &self.query.iter().collect::<String>(),
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let mut matches: Vec<_> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let name_score = self
                    .pattern
                    .score(entry.name_text.slice(..), &mut self.matcher);
                let score = name_score.or_else(|| {
                    self.pattern
                        .score(entry.path_text.slice(..), &mut self.matcher)
                })?;
                // Frecency lifts the places you actually go; it refines the
                // fuzzy ranking rather than overriding a clearly better match.
                Some((
                    index,
                    name_score.is_some(),
                    score.saturating_add(entry.boost),
                    entry.activity,
                ))
            })
            .collect();
        matches.sort_by_key(|&(index, name_match, score, activity)| {
            (
                Reverse(name_match),
                Reverse(score),
                Reverse(activity),
                index,
            )
        });
        self.matches = matches.into_iter().map(|(index, ..)| index).collect();
        self.selection =
            ListState::default().with_selected((!self.matches.is_empty()).then_some(0));
    }

    fn selected(&self) -> Option<&Entry> {
        self.selection
            .selected()
            .and_then(|index| self.matches.get(index))
            .and_then(|index| self.entries.get(*index))
    }

    fn move_selection(&mut self, forward: bool, amount: usize) {
        if self.matches.is_empty() {
            return;
        }
        let current = self.selection.selected().unwrap_or_default();
        let next = if forward {
            current
                .saturating_add(amount)
                .min(self.matches.len().saturating_sub(1))
        } else {
            current.saturating_sub(amount)
        };
        self.selection.select(Some(next));
    }

    fn insert(&mut self, text: &str) {
        for character in text.chars().filter(|character| !character.is_control()) {
            if self.query.len() >= MAX_QUERY {
                break;
            }
            self.query.insert(self.cursor, character);
            self.cursor = self.cursor.saturating_add(1);
        }
        self.filter();
    }

    fn key(&mut self, key: KeyEvent, page: usize) -> Action {
        if key.kind == KeyEventKind::Release {
            return Action::Continue;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('c') => return Action::Cancel,
                KeyCode::Char('z') => return Action::Suspend,
                KeyCode::Char('d') if self.query.is_empty() => return Action::Cancel,
                KeyCode::Char('n') => self.move_selection(true, 1),
                KeyCode::Char('p') => self.move_selection(false, 1),
                KeyCode::Char('a') => self.cursor = 0,
                KeyCode::Char('e') => self.cursor = self.query.len(),
                KeyCode::Char('u') => {
                    self.query.clear();
                    self.cursor = 0;
                    self.filter();
                }
                KeyCode::Char('w') => {
                    while self.cursor > 0
                        && self
                            .query
                            .get(self.cursor.saturating_sub(1))
                            .is_some_and(|character| character.is_whitespace())
                    {
                        self.cursor = self.cursor.saturating_sub(1);
                        self.query.remove(self.cursor);
                    }
                    while self.cursor > 0
                        && self
                            .query
                            .get(self.cursor.saturating_sub(1))
                            .is_some_and(|character| !character.is_whitespace())
                    {
                        self.cursor = self.cursor.saturating_sub(1);
                        self.query.remove(self.cursor);
                    }
                    self.filter();
                }
                _ => {}
            }
            return Action::Continue;
        }
        match key.code {
            KeyCode::Esc => return Action::Cancel,
            KeyCode::Enter => {
                return self
                    .selected()
                    .map_or(Action::Continue, |entry| Action::Select(entry.path.clone()));
            }
            KeyCode::Down | KeyCode::Tab => self.move_selection(true, 1),
            KeyCode::Up | KeyCode::BackTab => self.move_selection(false, 1),
            KeyCode::PageDown => self.move_selection(true, page),
            KeyCode::PageUp => self.move_selection(false, page),
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = self.cursor.saturating_add(1).min(self.query.len()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.query.len(),
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor = self.cursor.saturating_sub(1);
                self.query.remove(self.cursor);
                self.filter();
            }
            KeyCode::Delete if self.cursor < self.query.len() => {
                self.query.remove(self.cursor);
                self.filter();
            }
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::ALT) => {
                self.insert(&character.to_string());
            }
            _ => {}
        }
        Action::Continue
    }

    fn accent(&self) -> Style {
        if self.monochrome {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
        }
    }

    fn result_line(&mut self, entry_index: usize, selected: bool) -> Line<'static> {
        let Some(entry) = self.entries.get(entry_index) else {
            return Line::default();
        };
        let accent = self.accent();
        let mut indices = Vec::new();
        self.pattern
            .indices(entry.path_text.slice(..), &mut self.matcher, &mut indices);
        // Everything before the repository name is context; the name carries the weight.
        let leaf_start = entry
            .display
            .chars()
            .count()
            .saturating_sub(entry.name.chars().count());
        let base = |position: usize| {
            let style = if position < leaf_start {
                Style::default().add_modifier(Modifier::DIM)
            } else {
                Style::default()
            };
            if selected {
                style.add_modifier(Modifier::BOLD)
            } else {
                style
            }
        };
        let mut spans = vec![if selected {
            Span::styled("› ", accent)
        } else {
            Span::raw("  ")
        }];
        spans.extend(
            entry
                .display
                .chars()
                .enumerate()
                .map(|(position, character)| {
                    let matched =
                        u32::try_from(position).is_ok_and(|position| indices.contains(&position));
                    Span::styled(
                        character.to_string(),
                        if matched { accent } else { base(position) },
                    )
                }),
        );
        let line = Line::from(spans);
        if selected && !self.monochrome {
            line.style(Style::default().bg(Color::Indexed(236)))
        } else {
            line
        }
    }

    /// The selected repository, when its preview has not been loaded yet.
    fn wanted_preview(&self) -> Option<PathBuf> {
        self.selected()
            .map(|entry| entry.path.clone())
            .filter(|path| !self.previews.contains_key(path))
    }

    fn preview_lines(&self, width: usize) -> Vec<Line<'static>> {
        let muted = Style::default().add_modifier(Modifier::DIM);
        let Some(entry) = self.selected() else {
            return Vec::new();
        };
        let fit = |text: String| crate::ui::truncate(&text, width);
        let mut lines = vec![Line::styled(
            fit(entry.display.clone()),
            Style::default().add_modifier(Modifier::BOLD),
        )];
        let Some(info) = self.previews.get(&entry.path) else {
            lines.push(Line::styled("reading…", muted));
            return lines;
        };
        if info.unreadable {
            lines.push(Line::styled("Git could not read this checkout", muted));
            return lines;
        }
        lines.push(Line::raw(fit(info::branch_line(info))));
        let changed = if self.monochrome {
            Style::default()
        } else {
            Style::default().fg(Color::Yellow)
        };
        lines.push(info::changes(info).map_or_else(
            || Line::styled("clean", muted),
            |changes| Line::styled(fit(changes), changed),
        ));
        lines.push(Line::styled(
            fit(info.remote.clone().unwrap_or_else(|| "no remote".into())),
            muted,
        ));
        lines.push(Line::styled(fit(info::visit_line(info)), muted));
        for commit in &info.commits {
            lines.push(Line::from(vec![
                Span::styled(format!("{} ", commit.hash), muted),
                Span::raw(crate::ui::truncate(
                    &format!("{}  {}", crate::visits::ago(commit.time), commit.subject),
                    width.saturating_sub(commit.hash.chars().count().saturating_add(1)),
                )),
            ]));
        }
        lines
    }

    fn draw_preview(&self, frame: &mut Frame<'_>, area: ratatui::layout::Rect) {
        let lines = self.preview_lines(usize::from(area.width.saturating_sub(3)));
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::LEFT)
                    .border_style(Style::default().add_modifier(Modifier::DIM))
                    .padding(Padding::horizontal(1)),
            ),
            area,
        );
    }

    fn draw(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
        if area.width == 0 || area.height == 0 {
            return;
        }
        let footer_rows = u16::from(area.height >= 4);
        let [prompt, results, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(footer_rows),
        ])
        .areas(area);
        let accent = self.accent();
        let muted = Style::default().add_modifier(Modifier::DIM);

        // Prompt with the running count on the right.
        let counter = format!(" {}/{} ", self.matches.len(), self.entries.len());
        let counter_width = u16::try_from(counter.chars().count()).unwrap_or(u16::MAX);
        let [input, count] =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(counter_width)])
                .areas(prompt);
        let label = "repot ❯ ";
        let label_width = u16::try_from(Line::from(label).width()).unwrap_or(u16::MAX);
        let query: String = self.query.iter().collect();
        let before: String = self.query.iter().take(self.cursor).collect();
        let cursor_width = Line::from(before).width();
        let available = usize::from(input.width.saturating_sub(label_width).saturating_sub(1));
        let scroll = cursor_width.saturating_sub(available);
        let visible_query: String = {
            let mut skipped = 0_usize;
            query
                .chars()
                .skip_while(|character| {
                    let size = Line::from(character.to_string()).width();
                    if skipped.saturating_add(size) <= scroll {
                        skipped = skipped.saturating_add(size);
                        true
                    } else {
                        false
                    }
                })
                .collect()
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(label, accent),
                Span::raw(visible_query),
            ])),
            input,
        );
        frame.render_widget(Paragraph::new(counter).style(muted), count);
        frame.set_cursor_position((
            input
                .x
                .saturating_add(label_width)
                .saturating_add(
                    u16::try_from(cursor_width.saturating_sub(scroll)).unwrap_or_default(),
                )
                .min(input.right().saturating_sub(1)),
            input.y,
        ));

        // Results, best match first, directly under the prompt, with details
        // of the selection beside them when there is room.
        let results = if area.width >= PREVIEW_WIDTH {
            let [list, preview] =
                Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
                    .areas(results);
            self.draw_preview(frame, preview);
            list
        } else {
            results
        };
        let rows = usize::from(results.height);
        if self.matches.is_empty() {
            if rows > 0 {
                frame.render_widget(Paragraph::new(Line::styled("  no matches", muted)), results);
            }
        } else if rows > 0 {
            let selected = self.selection.selected().unwrap_or_default();
            let offset = self
                .selection
                .offset()
                .min(selected)
                .max(selected.saturating_add(1).saturating_sub(rows));
            *self.selection.offset_mut() = offset;
            let visible = self.matches.clone();
            let lines: Vec<Line<'static>> = visible
                .into_iter()
                .enumerate()
                .skip(offset)
                .take(rows)
                .map(|(position, entry)| self.result_line(entry, position == selected))
                .collect();
            frame.render_widget(Paragraph::new(lines), results);
        }

        if footer_rows > 0 {
            let help = if area.width >= 56 {
                "  ↑↓ move · enter cd · esc cancel · ctrl-u clear"
            } else {
                "  esc cancel"
            };
            frame.render_widget(Paragraph::new(help).style(muted), footer);
        }
    }
}

/// Raw mode and bracketed paste for the lifetime of the picker, undone on
/// every exit path including panics and signals.
struct TerminalGuard(File);

impl TerminalGuard {
    fn restore(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.0, DisableBracketedPaste, Show);
    }

    fn enter(&mut self) -> io::Result<()> {
        enable_raw_mode()?;
        execute!(self.0, EnableBracketedPaste)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

/// Crossterm writes its cursor position query to stdout. Point stdout at the
/// terminal while the picker runs so the query can never leak into a pipe or
/// the handoff output, and restore it before the selection is printed.
struct StdoutOnTerminal(Option<OwnedFd>);

impl StdoutOnTerminal {
    fn new(terminal: &File) -> io::Result<Self> {
        io::stdout().flush()?;
        let saved = rustix::io::fcntl_dupfd_cloexec(io::stdout().as_fd(), 0)?;
        rustix::stdio::dup2_stdout(terminal)?;
        Ok(Self(Some(saved)))
    }
}

impl Drop for StdoutOnTerminal {
    fn drop(&mut self) {
        let _ = io::stdout().flush();
        if let Some(saved) = self.0.take() {
            let _ = rustix::stdio::dup2_stdout(&saved);
        }
    }
}

type Screen = Terminal<CrosstermBackend<File>>;

/// Anchor an inline viewport at the cursor; the rest of the scrollback stays visible.
fn screen(guard: &TerminalGuard, height: u16) -> io::Result<Screen> {
    let rows = crossterm::terminal::size().map_or(height, |(_, rows)| rows);
    Terminal::with_options(
        CrosstermBackend::new(guard.0.try_clone()?),
        TerminalOptions {
            viewport: Viewport::Inline(height.min(rows).max(1)),
        },
    )
}

/// Erase the picker and leave the cursor where it started, so the shell prompt
/// returns to the same place, as if the picker had never been there.
fn erase(screen: &mut Screen) {
    let area = screen.get_frame().area();
    let _ = screen.set_cursor_position(area.as_position());
    let _ = execute!(
        screen.backend_mut(),
        crossterm::cursor::MoveTo(0, area.y),
        crossterm::terminal::Clear(crossterm::terminal::ClearType::FromCursorDown)
    );
}

struct Signals {
    cancel: Arc<AtomicBool>,
    suspend: Arc<AtomicBool>,
    registrations: Vec<signal_hook::SigId>,
}

impl Signals {
    fn new() -> io::Result<Self> {
        let mut signals = Self {
            cancel: Arc::new(AtomicBool::new(false)),
            suspend: Arc::new(AtomicBool::new(false)),
            registrations: Vec::new(),
        };
        for signal in [
            signal_hook::consts::SIGINT,
            signal_hook::consts::SIGTERM,
            signal_hook::consts::SIGHUP,
        ] {
            signals.registrations.push(signal_hook::flag::register(
                signal,
                Arc::clone(&signals.cancel),
            )?);
        }
        signals.registrations.push(signal_hook::flag::register(
            signal_hook::consts::SIGTSTP,
            Arc::clone(&signals.suspend),
        )?);
        Ok(signals)
    }
}

impl Drop for Signals {
    fn drop(&mut self) {
        for registration in &self.registrations {
            signal_hook::low_level::unregister(*registration);
        }
    }
}

pub(super) fn pick(repositories: &[Repository], roots: &[PathBuf], query: &str) -> Result<PathBuf> {
    run(repositories, roots, query)
        .map_err(|error| format!("repository picker: {error}"))?
        .ok_or_else(|| "repository selection cancelled".into())
}

fn run(repositories: &[Repository], roots: &[PathBuf], query: &str) -> io::Result<Option<PathBuf>> {
    // The shell integration may pipe stdout and stderr. The UI belongs to the
    // controlling terminal; only the chosen path belongs to the output stream.
    let terminal = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
    let signals = Signals::new()?;
    let _stdout = StdoutOnTerminal::new(&terminal)?;
    let mut guard = TerminalGuard(terminal);
    guard.enter()?;
    // Prompt and key hint around the results, never taller than needed, but
    // tall enough for the preview when it is shown.
    let wide = crossterm::terminal::size().is_ok_and(|(columns, _)| columns >= PREVIEW_WIDTH);
    let height = u16::try_from(repositories.len().saturating_add(2))
        .unwrap_or(u16::MAX)
        .max(if wide { 9 } else { 0 })
        .min(HEIGHT);
    let mut screen = screen(&guard, height)?;
    let visits = crate::visits::load();
    let mut picker = Picker::new(
        repositories,
        roots,
        &visits,
        query,
        env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()),
    );
    // Previews read Git on a worker so typing never waits for a slow checkout.
    let (requests, pending) = mpsc::channel::<PathBuf>();
    let (finished, previews) = mpsc::channel::<(PathBuf, Info)>();
    std::thread::spawn(move || {
        while let Ok(path) = pending.recv() {
            let info = info::gather(&path, visits.get(&path));
            if finished.send((path, info)).is_err() {
                break;
            }
        }
    });
    let mut requested = HashSet::new();
    // Install the resize handler before exposing the first frame.
    event::poll(Duration::ZERO)?;
    let outcome = loop {
        if let Some(path) = picker.wanted_preview()
            && requested.insert(path.clone())
        {
            let _ = requests.send(path);
        }
        let mut size = screen.draw(|frame| picker.draw(frame))?.area.as_size();
        let event = loop {
            if signals.cancel.load(Ordering::Relaxed) {
                break None;
            }
            let mut arrived = false;
            while let Ok((path, info)) = previews.try_recv() {
                picker.previews.insert(path, info);
                arrived = true;
            }
            if arrived {
                screen.draw(|frame| picker.draw(frame))?;
            }
            if signals.suspend.swap(false, Ordering::Relaxed) {
                suspend(&mut guard, &mut screen, height)?;
                screen.draw(|frame| picker.draw(frame))?;
            }
            let current_size = screen.size()?;
            if current_size != size {
                screen.draw(|frame| picker.draw(frame))?;
                size = current_size;
            }
            if event::poll(Duration::from_millis(100))? {
                break Some(event::read()?);
            }
        };
        let Some(event) = event else {
            break None;
        };
        let page = usize::from(height.saturating_sub(2)).max(1);
        match event {
            Event::Key(key) => match picker.key(key, page) {
                Action::Select(path) => break Some(path),
                Action::Cancel => break None,
                Action::Suspend => suspend(&mut guard, &mut screen, height)?,
                Action::Continue => {}
            },
            Event::Paste(text) => picker.insert(&text),
            _ => {}
        }
    };
    erase(&mut screen);
    Ok(outcome)
}

fn suspend(guard: &mut TerminalGuard, screen: &mut Screen, height: u16) -> io::Result<()> {
    erase(screen);
    guard.restore();
    nix::sys::signal::raise(nix::sys::signal::Signal::SIGSTOP).map_err(io::Error::other)?;
    guard.enter()?;
    *screen = self::screen(guard, height)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn picker(query: &str) -> Picker {
        let repositories = [
            "/projects/alpha",
            "/projects/beta",
            "/other/alpha",
            "/projects/café",
            "/projects/日本語",
            "/projects/space\nname",
        ]
        .map(|path| Repository {
            path: PathBuf::from(path),
        });
        Picker::new(
            &repositories,
            &[PathBuf::from("/projects")],
            &HashMap::new(),
            query,
            true,
        )
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn frecency_orders_the_empty_query_and_refines_but_never_hides_matches() {
        let repositories =
            ["/projects/alpha", "/projects/beta", "/projects/gamma"].map(|path| Repository {
                path: PathBuf::from(path),
            });
        let visited = |path: &str, frecency| {
            (
                PathBuf::from(path),
                Visits {
                    count: 1,
                    last: Some(1),
                    frecency,
                },
            )
        };
        let visits = HashMap::from([
            visited("/projects/gamma", 160),
            visited("/projects/beta", 16),
        ]);
        let picker = Picker::new(
            &repositories,
            &[PathBuf::from("/projects")],
            &visits,
            "",
            true,
        );
        assert_eq!(picker.matches, [2, 1, 0], "most used first, then by visits");
        let picker = Picker::new(
            &repositories,
            &[PathBuf::from("/projects")],
            &visits,
            "alp",
            true,
        );
        assert_eq!(
            picker.matches,
            [0],
            "frecency never adds non-matching entries"
        );
    }

    #[test]
    fn initial_query_ranking_ties_and_no_match_are_deterministic() {
        let mut picker = picker("alp");
        assert_eq!(picker.matches, [0, 2]);
        assert_eq!(picker.key(key(KeyCode::Down), 8), Action::Continue);
        assert_eq!(
            picker.key(key(KeyCode::Enter), 8),
            Action::Select(PathBuf::from("/other/alpha"))
        );
        picker.insert("zzz");
        assert!(picker.selected().is_none());
        assert_eq!(picker.key(key(KeyCode::Enter), 8), Action::Continue);
        picker.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL), 8);
        assert_eq!(picker.matches, [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn unicode_edits_paste_and_cancellation_preserve_paths() {
        let mut picker = picker("日本語");
        picker.key(key(KeyCode::Left), 4);
        picker.key(key(KeyCode::Backspace), 4);
        picker.insert("本");
        assert_eq!(
            picker.key(key(KeyCode::Enter), 4),
            Action::Select(PathBuf::from("/projects/日本語"))
        );
        picker.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL), 4);
        picker.insert("space\u{1b}\n");
        assert_eq!(picker.query.iter().collect::<String>(), "space");
        assert_eq!(
            picker.key(key(KeyCode::Enter), 4),
            Action::Select(PathBuf::from("/projects/space\nname"))
        );
        assert_eq!(picker.key(key(KeyCode::Esc), 4), Action::Cancel);
        assert_eq!(
            picker.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL), 4),
            Action::Cancel
        );
        assert_eq!(
            visible("a\n\u{1b}]52;c;injected\u{7}"),
            "a\\n\\u{1b}]52;c;injected\\u{7}"
        );
    }

    #[test]
    fn rendering_resizes_and_empty_results_without_losing_selection() {
        let mut picker = picker("");
        for (width, height) in [(80, 24), (120, 40), (200, 60), (24, 8), (1, 1), (0, 0)] {
            let mut terminal =
                Terminal::new(TestBackend::new(width, height)).expect("test terminal");
            terminal.draw(|frame| picker.draw(frame)).expect("render");
            if width >= 80 {
                let text: String = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(ratatui::buffer::Cell::symbol)
                    .collect();
                assert!(text.contains("repot ❯"));
                assert!(text.contains("enter cd"));
                assert!(text.contains("6/6"));
                assert!(!text.contains('\u{1b}'));
            }
        }
        picker.insert("nomatch");
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test terminal");
        terminal
            .draw(|frame| picker.draw(frame))
            .expect("render empty");
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(text.contains("no matches"));
    }

    #[test]
    fn seeded_input_simulation_keeps_cursor_and_selection_valid() {
        let keys = [
            key(KeyCode::Char('a')),
            key(KeyCode::Char('é')),
            key(KeyCode::Char('日')),
            key(KeyCode::Backspace),
            key(KeyCode::Delete),
            key(KeyCode::Left),
            key(KeyCode::Right),
            key(KeyCode::Home),
            key(KeyCode::End),
            key(KeyCode::Up),
            key(KeyCode::Down),
            key(KeyCode::PageDown),
            key(KeyCode::PageUp),
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
        ];
        for seed in 0_u64..64 {
            let mut random = seed;
            let mut state = picker("");
            let mut replay = picker("");
            for _ in 0..512 {
                random = random
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                let index = usize::try_from(random % u64::try_from(keys.len()).expect("key count"))
                    .expect("key index");
                let event = keys[index];
                assert_eq!(state.key(event, 5), replay.key(event, 5), "seed {seed}");
                assert!(state.cursor <= state.query.len(), "seed {seed}");
                assert!(
                    state
                        .selection
                        .selected()
                        .is_none_or(|index| index < state.matches.len()),
                    "seed {seed}"
                );
                assert_eq!(state.matches, replay.matches, "seed {seed}");
            }
        }
    }
}
