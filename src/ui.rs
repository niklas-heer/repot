//! Human-facing terminal presentation: colour, progress and diagnostics.
//!
//! Machine-readable output (`--json`, piped plain text) never goes through the
//! styling here, and progress only ever appears on an interactive stderr.

use std::fmt::{Display, Write as _};
use std::io::{IsTerminal, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub use clap::builder::styling::Style;
use clap::builder::styling::{Ansi256Color, AnsiColor, Effects};

pub const BOLD: Style = Style::new().effects(Effects::BOLD);
pub const DIM: Style = Style::new().effects(Effects::DIMMED);
pub const GOOD: Style =
    Style::new().fg_color(Some(clap::builder::styling::Color::Ansi(AnsiColor::Green)));
pub const WARN: Style =
    Style::new().fg_color(Some(clap::builder::styling::Color::Ansi(AnsiColor::Yellow)));
pub const BAD: Style =
    Style::new().fg_color(Some(clap::builder::styling::Color::Ansi(AnsiColor::Red)));
pub const INFO: Style =
    Style::new().fg_color(Some(clap::builder::styling::Color::Ansi(AnsiColor::Cyan)));
pub const PUSH: Style = Style::new().fg_color(Some(clap::builder::styling::Color::Ansi(
    AnsiColor::Magenta,
)));
pub const LEAF: Style = Style::new().fg_color(Some(clap::builder::styling::Color::Ansi256(
    Ansi256Color(71),
)));
pub const POT: Style = Style::new().fg_color(Some(clap::builder::styling::Color::Ansi256(
    Ansi256Color(166),
)));

/// Whether a stream should receive ANSI colour, following the `NO_COLOR` and
/// `CLICOLOR_FORCE` conventions.
fn colour(terminal: bool) -> bool {
    let set = |name| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    if set("NO_COLOR") {
        return false;
    }
    if std::env::var_os("CLICOLOR_FORCE").is_some_and(|value| !value.is_empty() && value != "0") {
        return true;
    }
    terminal && std::env::var_os("TERM").is_none_or(|term| term != "dumb")
}

/// Styling decisions for one output stream.
#[derive(Clone, Copy)]
pub struct Paint {
    colour: bool,
}

impl Paint {
    pub fn stdout() -> Self {
        Self {
            colour: colour(std::io::stdout().is_terminal()),
        }
    }

    pub fn stderr() -> Self {
        Self {
            colour: colour(std::io::stderr().is_terminal()),
        }
    }

    pub const fn colour(self) -> bool {
        self.colour
    }

    pub fn paint(self, style: Style, text: impl Display) -> String {
        if self.colour {
            format!("{style}{text}{style:#}")
        } else {
            text.to_string()
        }
    }
}

/// Escape terminal controls so repository names and paths cannot move the
/// cursor, change colours or inject escape sequences.
pub fn visible(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| {
            if character.is_control() {
                character.escape_default().collect::<Vec<_>>()
            } else {
                vec![character]
            }
        })
        .collect()
}

pub fn width(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}

/// Shorten text to a display width, marking the cut with an ellipsis.
pub fn truncate(text: &str, limit: usize) -> String {
    if width(text) <= limit {
        return text.to_owned();
    }
    let mut result = String::new();
    let mut used = 0_usize;
    for character in text.chars() {
        let size = unicode_width::UnicodeWidthChar::width(character).unwrap_or_default();
        if used.saturating_add(size) >= limit {
            break;
        }
        used = used.saturating_add(size);
        result.push(character);
    }
    result.push('…');
    result
}

/// Pad text to a display width; styling is applied afterwards so escape codes
/// never distort alignment.
pub fn pad(text: &str, target: usize) -> String {
    let mut padded = text.to_owned();
    for _ in width(text)..target {
        padded.push(' ');
    }
    padded
}

pub fn terminal_width() -> usize {
    crossterm::terminal::size()
        .ok()
        .map_or(100, |(columns, _)| usize::from(columns))
        .clamp(40, 160)
}

/// Present a checkout path the way people talk about it: relative to its
/// repository root, without the default `github.com` host, or under `~`.
pub fn repository_name(path: &Path, roots: &[std::path::PathBuf]) -> String {
    for root in roots {
        let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
        if let Ok(relative) = path
            .strip_prefix(&canonical)
            .or_else(|_| path.strip_prefix(root))
        {
            let relative = relative.strip_prefix("github.com").unwrap_or(relative);
            return visible(&relative.to_string_lossy());
        }
    }
    if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from)
        && let Ok(relative) = path.strip_prefix(&home)
    {
        return visible(&Path::new("~").join(relative).to_string_lossy());
    }
    visible(&path.to_string_lossy())
}

/// Print a failure the way every command reports one.
pub fn error(message: &str) {
    let paint = Paint::stderr();
    let (message, hint) = message
        .split_once("; ")
        .map_or((message, None), |(message, hint)| (message, Some(hint)));
    let mut text = format!("{} {message}\n", paint.paint(BAD.bold(), "error:"));
    if let Some(hint) = hint {
        let _ = writeln!(text, "  {} {hint}", paint.paint(DIM, "hint:"));
    }
    let _ = std::io::stderr().lock().write_all(text.as_bytes());
}

/// Confirm a completed change on stderr.
pub fn done(message: &str) {
    note("✓", GOOD, message);
}

/// A short status note on stderr, such as where a checkout went.
pub fn note(symbol: &str, style: Style, message: &str) {
    let paint = Paint::stderr();
    let _ = writeln!(
        std::io::stderr().lock(),
        "{} {message}",
        paint.paint(style, symbol)
    );
}

const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

struct ProgressState {
    label: Mutex<String>,
    current: Mutex<String>,
    done: AtomicUsize,
    total: AtomicUsize,
    stop: AtomicBool,
}

/// A single-line spinner with a counter on an interactive stderr.
///
/// It redraws with carriage returns and spaces only, so terminals that ignore
/// escape sequences still show a clean line, and it clears itself when dropped.
pub struct Progress {
    state: Option<Arc<ProgressState>>,
    thread: Option<JoinHandle<()>>,
}

impl Progress {
    pub fn start(label: &str, total: usize) -> Self {
        let interactive = std::io::stderr().is_terminal()
            && std::env::var_os("TERM").is_none_or(|term| term != "dumb");
        if !interactive {
            return Self {
                state: None,
                thread: None,
            };
        }
        let state = Arc::new(ProgressState {
            label: Mutex::new(label.to_owned()),
            current: Mutex::new(String::new()),
            done: AtomicUsize::new(0),
            total: AtomicUsize::new(total),
            stop: AtomicBool::new(false),
        });
        let shared = Arc::clone(&state);
        let thread = std::thread::spawn(move || spin(&shared));
        Self {
            state: Some(state),
            thread: Some(thread),
        }
    }

    pub fn phase(&self, label: &str, total: usize) {
        if let Some(state) = &self.state {
            label.clone_into(&mut state.label.lock().unwrap_or_else(PoisonError::into_inner));
            state.done.store(0, Ordering::Relaxed);
            state.total.store(total, Ordering::Relaxed);
        }
    }

    pub fn working_on(&self, item: &str) {
        if let Some(state) = &self.state {
            item.clone_into(&mut state.current.lock().unwrap_or_else(PoisonError::into_inner));
        }
    }

    pub fn advance(&self) {
        if let Some(state) = &self.state {
            state.done.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        if let Some(state) = &self.state {
            state.stop.store(true, Ordering::Relaxed);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn spin(state: &ProgressState) {
    // Fast commands finish before the first frame and never flicker.
    let started = Instant::now();
    let mut drawn = 0_usize;
    let mut frames = FRAMES.iter().cycle();
    let paint = Paint::stderr();
    while !state.stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(80));
        if started.elapsed() < Duration::from_millis(150) {
            continue;
        }
        let symbol = frames.next().copied().unwrap_or("·");
        let label = state
            .label
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let current = state
            .current
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let counter = format!(
            "{}/{}",
            state.done.load(Ordering::Relaxed),
            state.total.load(Ordering::Relaxed)
        );
        let room = terminal_width().saturating_sub(
            width(&label)
                .saturating_add(width(&counter))
                .saturating_add(6),
        );
        let current = truncate(&current, room);
        let plain = format!("{symbol} {label} {counter}  {current}");
        let line = format!(
            "{} {} {}  {}",
            paint.paint(GOOD, symbol),
            label,
            paint.paint(BOLD, &counter),
            paint.paint(DIM, &current)
        );
        let clear = " ".repeat(drawn.saturating_sub(width(&plain)));
        drawn = width(&plain);
        let _ = write!(std::io::stderr().lock(), "\r{line}{clear}");
    }
    if drawn > 0 {
        let _ = write!(
            std::io::stderr().lock(),
            "\r{}\r",
            " ".repeat(drawn.saturating_add(1))
        );
    }
}
