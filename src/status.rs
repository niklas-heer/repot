//! Repository observations and conservative, reusable update plans.

use std::ffi::OsStr;
use std::fmt::Write as _;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::Result;
use crate::config::{Config, remote_parts};
use crate::discovery;
use crate::process;
use crate::ui;

pub struct Options {
    pub json: bool,
    pub jobs: usize,
    pub timeout: Duration,
    pub no_fetch: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Dirty {
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
}

impl Dirty {
    const fn clean(&self) -> bool {
        self.staged == 0 && self.unstaged == 0 && self.untracked == 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub head: String,
    pub target: String,
    pub reference: String,
    pub branch: Option<String>,
    pub previous_default: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub path: PathBuf,
    pub state: String,
    pub action: String,
    pub branch: Option<String>,
    pub dirty: Dirty,
    pub stashes: usize,
    pub ahead: usize,
    pub behind: usize,
    pub reason: String,
    pub applied: bool,
    #[serde(skip)]
    pub failed: bool,
    #[serde(skip)]
    pub plan: Option<Plan>,
    /// The remote the branch tracks, for suggesting exact commands.
    #[serde(skip)]
    pub remote: Option<String>,
}

impl Report {
    fn empty(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            state: "no-remote".into(),
            action: "review".into(),
            branch: None,
            dirty: Dirty::default(),
            stashes: 0,
            ahead: 0,
            behind: 0,
            reason: String::new(),
            applied: false,
            failed: false,
            plan: None,
            remote: None,
        }
    }

    pub fn fail(&mut self, reason: &str) {
        self.failed = true;
        self.action = "review".into();
        self.reason = reason.into();
        self.plan = None;
    }
}

pub fn run(config: &Config, options: &Options) -> Result<u8> {
    let started = std::time::Instant::now();
    let reports = {
        let progress = (!options.json).then(|| {
            ui::Progress::start(
                if options.no_fetch {
                    "Inspecting"
                } else {
                    "Fetching"
                },
                0,
            )
        });
        collect(config, options, progress.as_ref(), None)?
    };
    render(
        &reports,
        options.json,
        &View {
            roots: &config.roots,
            mode: if options.no_fetch {
                Mode::Cached
            } else {
                Mode::Status
            },
            elapsed: started.elapsed(),
        },
    )
}

/// Runs after each inspection on the same worker, so updates overlap with the
/// fetches of other repositories instead of waiting for all of them.
pub type Finish<'a> = &'a (dyn Fn(&mut Report) + Sync);

pub fn collect(
    config: &Config,
    options: &Options,
    progress: Option<&ui::Progress>,
    finish: Option<Finish<'_>>,
) -> Result<Vec<Report>> {
    let repositories = discovery::discover(config)?;
    if let Some(progress) = progress {
        progress.phase(
            match (options.no_fetch, finish.is_some()) {
                (true, _) => "Inspecting",
                (false, false) => "Fetching",
                (false, true) => "Syncing",
            },
            repositories.len(),
        );
    }
    // A shared queue keeps every worker busy even when one remote is slow.
    let next = std::sync::atomic::AtomicUsize::new(0);
    let workers = options.jobs.clamp(1, 32).min(repositories.len().max(1));
    let mut reports = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut reports = Vec::new();
                    while let Some(repo) =
                        repositories.get(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
                    {
                        if let Some(progress) = progress {
                            progress.working_on(&ui::repository_name(&repo.path, &config.roots));
                        }
                        let mut report = inspect(&repo.path, config, options, !options.no_fetch)
                            .unwrap_or_else(|_| {
                                let mut report = Report::empty(&repo.path);
                                report.state = "inspection-failed".into();
                                report.fail("could not inspect repository; inspect it with git");
                                report
                            });
                        if let Some(finish) = finish {
                            finish(&mut report);
                        }
                        reports.push(report);
                        if let Some(progress) = progress {
                            progress.advance();
                        }
                    }
                    reports
                })
            })
            .collect();
        let mut reports = Vec::new();
        for handle in handles {
            reports.extend(
                handle
                    .join()
                    .map_err(|_| "repository worker failed".to_owned())?,
            );
        }
        Ok::<_, String>(reports)
    })?;
    reports.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(reports)
}

/// What produced a set of reports, which changes how results are described.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Status,
    Cached,
    Sync,
    SyncPlan,
}

pub struct View<'a> {
    pub roots: &'a [PathBuf],
    pub mode: Mode,
    pub elapsed: Duration,
}

pub fn render(reports: &[Report], json: bool, view: &View<'_>) -> Result<u8> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(reports).map_err(|error| error.to_string())?
        );
    } else if std::io::stdout().is_terminal() {
        print!("{}", terminal_report(reports, view));
    } else {
        for report in reports {
            println!(
                "{}\t{}\t{}\tdirty={}/{}/{} stashes={}\t{}{}",
                report.action,
                report.state,
                report.path.display(),
                report.dirty.staged,
                report.dirty.unstaged,
                report.dirty.untracked,
                report.stashes,
                if report.applied { "done: " } else { "" },
                report.reason
            );
        }
    }
    Ok(if reports.iter().any(|report| report.failed) {
        1
    } else if reports
        .iter()
        .any(|report| matches!(report.action.as_str(), "review" | "push"))
    {
        3
    } else {
        0
    })
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Group {
    Failed,
    HeldBack,
    Review,
    Push,
    Update,
    Updated,
    Working,
    Clean,
}

/// Reasons that only restate what the facts column already shows.
const GENERIC_REASONS: [&str; 2] = [
    "working tree has local changes",
    "review branch, upstream and stashes manually",
];

impl Group {
    /// Presentation only: JSON keeps the stable `action` values.
    fn of(report: &Report) -> Self {
        if report.failed {
            return Self::Failed;
        }
        if report.applied {
            return Self::Updated;
        }
        match report.action.as_str() {
            "pull" | "return" => Self::Update,
            "push" => Self::Push,
            "none" => Self::Clean,
            _ => {
                let dirty = !report.dirty.clean();
                let generic = GENERIC_REASONS.contains(&report.reason.as_str());
                match report.state.as_str() {
                    "behind" if dirty => Self::HeldBack,
                    // Nothing to fetch; the only thing going on is your own work.
                    "synced" | "ahead" if generic && (dirty || report.stashes > 0) => Self::Working,
                    _ => Self::Review,
                }
            }
        }
    }

    /// Advice shared by every row of a group, shown once beside its heading.
    const fn advice(self) -> Option<&'static str> {
        match self {
            Self::HeldBack => Some("commit or stash your changes, then run repot sync"),
            Self::Push => Some("push when ready; repot never pushes"),
            Self::Working => Some("uncommitted work; nothing new upstream"),
            Self::Failed => Some("left untouched"),
            _ => None,
        }
    }

    const fn heading(self, mode: Mode) -> (&'static str, &'static str, ui::Style) {
        match self {
            Self::Failed => ("✗", "Failed", ui::BAD),
            Self::HeldBack => ("◆", "Held back by local changes", ui::WARN),
            Self::Working => ("✎", "Work in progress", ui::INFO),
            Self::Review => ("●", "Needs review", ui::WARN),
            Self::Push => ("↑", "Ready to push", ui::PUSH),
            Self::Update => (
                "↓",
                match mode {
                    Mode::SyncPlan => "Would update",
                    _ => "Ready to update",
                },
                ui::INFO,
            ),
            Self::Updated => ("✓", "Updated", ui::GOOD),
            Self::Clean => ("✓", "Up to date", ui::GOOD),
        }
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// Compact facts for one repository: position against upstream, then local work.
fn facts(report: &Report, paint: ui::Paint) -> Vec<String> {
    let mut facts = Vec::new();
    match report.state.as_str() {
        "ahead" | "behind" | "diverged" | "synced" => {
            if report.ahead > 0 {
                facts.push(paint.paint(ui::PUSH, format!("↑{}", report.ahead)));
            }
            if report.behind > 0 {
                facts.push(paint.paint(ui::INFO, format!("↓{}", report.behind)));
            }
        }
        "no-remote" => facts.push(paint.paint(ui::WARN, "no remote")),
        "no-upstream" => facts.push(paint.paint(ui::WARN, "no upstream")),
        "detached" => facts.push(paint.paint(ui::WARN, "detached HEAD")),
        "fetch-failed" => facts.push(paint.paint(ui::BAD, "fetch failed")),
        "inspection-failed" => facts.push(paint.paint(ui::BAD, "unreadable")),
        other => facts.push(ui::visible(other)),
    }
    if report.dirty.staged > 0 {
        facts.push(paint.paint(ui::GOOD, format!("{} staged", report.dirty.staged)));
    }
    if report.dirty.unstaged > 0 {
        facts.push(paint.paint(ui::WARN, format!("{} modified", report.dirty.unstaged)));
    }
    if report.dirty.untracked > 0 {
        facts.push(format!("{} untracked", report.dirty.untracked));
    }
    if report.stashes > 0 {
        facts.push(plural(report.stashes, "stash", "stashes"));
    }
    facts
}

/// The reason, when it adds something the facts and heading do not already say.
fn hint(report: &Report) -> Option<String> {
    if report.applied {
        return Some(match &report.plan {
            Some(Plan {
                branch: Some(branch),
                ..
            }) => format!("merged; switched to {branch}"),
            _ => "fast-forwarded".into(),
        });
    }
    let remote = report.remote.as_deref().unwrap_or("origin");
    let branch = report.branch.as_deref().map(ui::visible);
    match (report.state.as_str(), report.reason.as_str()) {
        (
            _,
            ""
            | "up to date"
            | "fast-forward current branch"
            | "local commits; push manually when ready",
        ) => None,
        (_, reason) if !GENERIC_REASONS.contains(&reason) && reason != "no remote configured" => {
            Some(ui::visible(reason))
        }
        ("no-remote", _) => Some("publish it with repot publish OWNER/NAME".into()),
        ("no-upstream", _) => {
            branch.map(|branch| format!("push it with git push -u {remote} {branch}"))
        }
        ("diverged", _) => Some("both sides moved; merge or rebase by hand".into()),
        ("detached", _) => Some("switch to a branch".into()),
        ("ahead", _) => Some("local commits not pushed yet".into()),
        ("behind", _) if report.stashes > 0 => Some("stashed work; review it first".into()),
        _ => None,
    }
}

/// Healthy checkouts are summarised by owner so attention goes where it is needed.
fn clean_summary(
    out: &mut String,
    members: &[usize],
    names: &[String],
    columns: usize,
    paint: ui::Paint,
) {
    let mut owners: Vec<(String, Vec<String>)> = Vec::new();
    for name in members.iter().filter_map(|index| names.get(*index)) {
        let (owner, leaf) = name.rsplit_once('/').map_or_else(
            || (String::new(), name.clone()),
            |(owner, leaf)| (format!("{owner}/"), leaf.to_owned()),
        );
        match owners.last_mut() {
            Some((last, leaves)) if *last == owner => leaves.push(leaf),
            _ => owners.push((owner, vec![leaf])),
        }
    }
    // Flow owners one after another; a new owner starts with its dimmed prefix.
    let mut line = String::from("   ");
    let mut length = 3_usize;
    for (owner, leaves) in owners {
        for (position, leaf) in leaves.iter().enumerate() {
            let prefix = if position == 0 { owner.as_str() } else { "" };
            let gap = match (length > 3, position) {
                (false, _) => 0,
                (true, 0) => 4,
                (true, _) => 2,
            };
            let size = ui::width(prefix).saturating_add(ui::width(leaf));
            if length > 3 && length.saturating_add(gap).saturating_add(size) > columns {
                let _ = writeln!(out, "{line}");
                line = String::from("   ");
                length = 3;
            } else {
                line.push_str(&" ".repeat(gap));
                length = length.saturating_add(gap);
            }
            let _ = write!(line, "{}{leaf}", paint.paint(ui::DIM, prefix));
            length = length.saturating_add(size);
        }
    }
    let _ = writeln!(out, "{line}");
    out.push('\n');
}

/// Timing, the source of the data, and the one next step worth taking.
fn footer(out: &mut String, reports: &[Report], view: &View<'_>, paint: ui::Paint) {
    let count = |group: Group| {
        reports
            .iter()
            .filter(|report| Group::of(report) == group)
            .count()
    };
    let seconds = view.elapsed.as_secs_f64();
    let action = match view.mode {
        Mode::Status => "fetched",
        Mode::Cached | Mode::SyncPlan => "inspected",
        Mode::Sync => "synced",
    };
    let mut summary = format!(
        " {} {action} in {seconds:.1}s",
        plural(reports.len(), "repository", "repositories")
    );
    if matches!(view.mode, Mode::Cached | Mode::SyncPlan) {
        summary.push_str(" from cached remote refs");
    }
    let _ = writeln!(out, "{}", paint.paint(ui::DIM, summary));
    let pending = count(Group::Update);
    let next = match view.mode {
        Mode::Status | Mode::Cached if pending > 0 => Some(format!(
            "run {} to update {}",
            paint.paint(ui::BOLD, "repot sync"),
            plural(pending, "repository", "repositories")
        )),
        Mode::SyncPlan if pending > 0 => Some(format!(
            "run {} to apply",
            paint.paint(ui::BOLD, "repot sync")
        )),
        _ if reports.is_empty() => Some(format!(
            "clone one with {}",
            paint.paint(ui::BOLD, "repot clone owner/name")
        )),
        _ => None,
    };
    if let Some(next) = next {
        let _ = writeln!(out, " {} {next}", paint.paint(ui::INFO, "→"));
    }
}

fn terminal_report(reports: &[Report], view: &View<'_>) -> String {
    let paint = ui::Paint::stdout();
    let columns = ui::terminal_width();
    let names: Vec<String> = reports
        .iter()
        .map(|report| ui::repository_name(&report.path, view.roots))
        .collect();
    let branches: Vec<String> = reports
        .iter()
        .map(|report| {
            report
                .branch
                .as_deref()
                .map_or_else(|| "—".to_owned(), ui::visible)
        })
        .collect();
    let listed = |report: &&Report| Group::of(report) != Group::Clean;
    let name_width = reports
        .iter()
        .zip(&names)
        .filter(|(report, _)| listed(report))
        .map(|(_, name)| ui::width(name))
        .max()
        .unwrap_or_default()
        .min(columns / 3);
    let branch_width = reports
        .iter()
        .zip(&branches)
        .filter(|(report, _)| listed(report))
        .map(|(_, branch)| ui::width(branch))
        .max()
        .unwrap_or_default()
        .min(18);
    let mut out = String::from("\n");
    let mut groups: Vec<Group> = reports.iter().map(Group::of).collect();
    groups.sort_unstable();
    groups.dedup();
    for group in groups {
        let members: Vec<usize> = (0..reports.len())
            .filter(|index| {
                reports
                    .get(*index)
                    .is_some_and(|report| Group::of(report) == group)
            })
            .collect();
        let (symbol, title, style) = group.heading(view.mode);
        let advice = group
            .advice()
            .map(|advice| format!("  {}", paint.paint(ui::DIM, format!("· {advice}"))))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            " {} {}  {}{advice}",
            paint.paint(style.bold(), symbol),
            paint.paint(ui::BOLD, title),
            paint.paint(ui::DIM, members.len())
        );
        if group == Group::Clean {
            clean_summary(&mut out, &members, &names, columns, paint);
            continue;
        }
        for index in members {
            let (Some(report), Some(name), Some(branch)) =
                (reports.get(index), names.get(index), branches.get(index))
            else {
                continue;
            };
            let name = ui::pad(&ui::truncate(name, name_width), name_width);
            let branch = ui::pad(&ui::truncate(branch, branch_width), branch_width);
            let facts = facts(report, paint).join(&paint.paint(ui::DIM, " · "));
            let mut line = format!(
                "   {}  {}  {}",
                paint.paint(ui::BOLD, name),
                paint.paint(ui::DIM, branch),
                facts
            );
            if let Some(hint) = hint(report) {
                let _ = write!(line, "  {}", paint.paint(ui::DIM, format!("· {hint}")));
            }
            let _ = writeln!(out, "{}", line.trim_end());
        }
        out.push('\n');
    }
    footer(&mut out, reports, view, paint);
    out
}

pub fn probe(path: &Path, args: &[&str], timeout: Duration) -> Result<Option<String>> {
    let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    let output = process::run("git", &args, path, timeout)?;
    if !output.success && output.code != Some(1) {
        return Err("Git inspection failed".into());
    }
    Ok(output.success.then(|| {
        output
            .stdout
            .strip_suffix('\n')
            .unwrap_or(&output.stdout)
            .to_owned()
    }))
}

fn required(path: &Path, args: &[&str], timeout: Duration) -> Result<String> {
    if timeout == Duration::from_secs(30) {
        return process::git(path, args);
    }
    probe(path, args, timeout)?.ok_or_else(|| "Git inspection failed".into())
}

pub fn inspect(path: &Path, config: &Config, options: &Options, fetch: bool) -> Result<Report> {
    let timeout = options.timeout;
    let mut report = Report::empty(path);
    report.branch = probe(
        path,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        timeout,
    )?;
    report.dirty = dirty(&required(
        path,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ],
        timeout,
    )?)?;
    report.stashes = required(path, &["stash", "list", "--format=%H"], timeout)?
        .lines()
        .count();
    let remotes = required(path, &["remote"], timeout)?;
    if remotes.is_empty() {
        report.reason = "no remote configured".into();
        return Ok(report);
    }
    let remote = choose_remote(path, report.branch.as_deref(), &remotes, timeout)?;
    report.remote = Some(remote.clone());
    if fetch
        && probe(
            path,
            &[
                "fetch",
                "--prune",
                "--no-tags",
                "--no-prune-tags",
                // Explicit refspecs alone still honor configured opportunistic
                // mappings, which can target local branches. Clear those too.
                "--refmap=",
                "--no-recurse-submodules",
                "--",
                &remote,
                &format!("+refs/heads/*:refs/remotes/{remote}/*"),
            ],
            timeout,
        )
        .ok()
        .flatten()
        .is_none()
    {
        report.state = "fetch-failed".into();
        report.fail("fetch failed or timed out; inspect the remote with git");
        return Ok(report);
    }
    let Some(branch) = report.branch.clone() else {
        report.state = "detached".into();
        report.reason = "detached HEAD; choose a branch manually".into();
        return Ok(report);
    };
    let Some(head) = probe(path, &["rev-parse", "--verify", "--quiet", "HEAD"], timeout)? else {
        report.state = "no-upstream".into();
        report.reason = "branch has no commits".into();
        return Ok(report);
    };
    let upstream = required(
        path,
        &[
            "for-each-ref",
            "--format=%(upstream)",
            &format!("refs/heads/{branch}"),
        ],
        timeout,
    )?;
    if !upstream.is_empty() && !upstream.starts_with(&format!("refs/remotes/{remote}/")) {
        report.state = "no-upstream".into();
        report.reason = "upstream is not a remote-tracking branch".into();
        return Ok(report);
    }
    let upstream_oid = if upstream.is_empty() {
        None
    } else {
        probe(
            path,
            &["rev-parse", "--verify", "--quiet", &upstream],
            timeout,
        )?
    };
    report.state = "no-upstream".into();
    if let Some(oid) = &upstream_oid {
        let counts = required(
            path,
            &[
                "rev-list",
                "--left-right",
                "--count",
                &format!("{head}...{oid}"),
            ],
            timeout,
        )?;
        let mut counts = counts.split_whitespace();
        report.ahead = counts
            .next()
            .ok_or("missing ahead count")?
            .parse()
            .map_err(|_| "invalid ahead count")?;
        report.behind = counts
            .next()
            .ok_or("missing behind count")?
            .parse()
            .map_err(|_| "invalid behind count")?;
        report.state = match (report.ahead, report.behind) {
            (0, 0) => "synced",
            (0, _) => "behind",
            (_, 0) => "ahead",
            _ => "diverged",
        }
        .into();
    }
    if !report.dirty.clean() {
        report.reason = "working tree has local changes".into();
        return Ok(report);
    }
    if let Some(reason) = blocked(path, timeout)? {
        report.reason = reason.into();
        return Ok(report);
    }
    match return_plan(
        path,
        &branch,
        &head,
        &remote,
        &upstream,
        upstream_oid.as_deref(),
        options,
        fetch,
    )? {
        Return::Plan(plan) => {
            if ignored_collision(path, &plan.target, timeout)? {
                report.reason = "ignored files would be overwritten".into();
                return Ok(report);
            }
            report.action = "return".into();
            report.reason = format!(
                "return to {} and fast-forward",
                plan.branch.as_deref().unwrap_or_default()
            );
            report.plan = Some(plan);
            return Ok(report);
        }
        Return::Review(reason) => {
            report.reason = reason.into();
            return Ok(report);
        }
        Return::None => {}
    }
    match report.state.as_str() {
        "synced" if report.stashes == 0 => {
            report.action = "none".into();
            report.reason = "up to date".into();
        }
        "behind" => {
            if let Some(target) = &upstream_oid
                && ignored_collision(path, target, timeout)?
            {
                report.reason = "ignored files would be overwritten".into();
                return Ok(report);
            }
            report.action = "pull".into();
            report.reason = "fast-forward current branch".into();
            report.plan = upstream_oid.map(|target| Plan {
                head,
                target,
                reference: upstream,
                branch: None,
                previous_default: None,
            });
        }
        "ahead" if owned(path, &remote, config, timeout)? => {
            report.action = "push".into();
            report.reason = "local commits; push manually when ready".into();
        }
        _ => {
            report.reason = "review branch, upstream and stashes manually".into();
        }
    }
    Ok(report)
}

fn choose_remote(
    path: &Path,
    branch: Option<&str>,
    remotes: &str,
    timeout: Duration,
) -> Result<String> {
    if let Some(branch) = branch
        && let Some(remote) = probe(
            path,
            &["config", "--get", &format!("branch.{branch}.remote")],
            timeout,
        )?
        && remote != "."
        && remotes.lines().any(|known| known == remote)
    {
        return Ok(remote);
    }
    Ok(if remotes.lines().any(|remote| remote == "origin") {
        "origin"
    } else {
        remotes.lines().next().ok_or("no remote")?
    }
    .into())
}

fn dirty(output: &str) -> Result<Dirty> {
    let mut result = Dirty::default();
    let mut records = output.split('\0').filter(|record| !record.is_empty());
    while let Some(record) = records.next() {
        let mut chars = record.chars();
        let staged = chars.next().ok_or("invalid status record")?;
        let unstaged = chars.next().ok_or("invalid status record")?;
        if chars.next() != Some(' ') {
            return Err("invalid status record".into());
        }
        if staged == '?' && unstaged == '?' {
            result.untracked = result.untracked.saturating_add(1);
        } else {
            if staged != ' ' {
                result.staged = result.staged.saturating_add(1);
            }
            if unstaged != ' ' {
                result.unstaged = result.unstaged.saturating_add(1);
            }
        }
        if matches!(staged, 'R' | 'C') || matches!(unstaged, 'R' | 'C') {
            records.next().ok_or("missing rename source")?;
        }
    }
    Ok(result)
}

fn blocked(path: &Path, timeout: Duration) -> Result<Option<&'static str>> {
    let flags = required(path, &["ls-files", "-v", "-z"], timeout)?;
    if flags.split('\0').any(|entry| {
        entry.starts_with('S')
            || entry
                .chars()
                .next()
                .is_some_and(|flag| flag.is_ascii_lowercase())
    }) {
        return Ok(Some("index flags can hide local edits; review manually"));
    }
    if let Some(branch) = probe(path, &["symbolic-ref", "--quiet", "HEAD"], timeout)? {
        let worktrees = required(path, &["worktree", "list", "--porcelain", "-z"], timeout)?;
        if worktrees
            .split('\0')
            .filter(|line| *line == format!("branch {branch}"))
            .count()
            > 1
        {
            return Ok(Some("current branch is checked out in another worktree"));
        }
    }
    match crate::git_read::Layout::open(path).and_then(|layout| layout.operation_in_progress()) {
        Some(true) => return Ok(Some("Git operation in progress")),
        Some(false) => {}
        None => {
            for marker in crate::git_read::OPERATION_MARKERS {
                let location = required(path, &["rev-parse", "--git-path", marker], timeout)?;
                if path.join(location).exists() {
                    return Ok(Some("Git operation in progress"));
                }
            }
        }
    }
    let entries = required(path, &["ls-files", "--stage", "-z"], timeout)?;
    if entries
        .split('\0')
        .any(|entry| entry.starts_with("160000 "))
    {
        return Ok(Some("submodules require manual review"));
    }
    Ok(None)
}

fn ignored_collision(path: &Path, target: &str, timeout: Duration) -> Result<bool> {
    let ignored = required(
        path,
        &[
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "-z",
        ],
        timeout,
    )?;
    if ignored.is_empty() {
        return Ok(false);
    }
    let tracked = required(
        path,
        &["ls-tree", "-r", "--name-only", "-z", target],
        timeout,
    )?;
    Ok(ignored
        .split('\0')
        .filter(|name| !name.is_empty())
        .any(|name| {
            tracked
                .split('\0')
                .filter(|entry| !entry.is_empty())
                .any(|entry| {
                    Path::new(name).starts_with(entry) || Path::new(entry).starts_with(name)
                })
        }))
}

fn owned(path: &Path, remote: &str, config: &Config, timeout: Duration) -> Result<bool> {
    let url = required(path, &["remote", "get-url", remote], timeout)?;
    Ok(remote_parts(&url).ok().is_some_and(|(_, parts)| {
        parts.len() >= 2
            && parts
                .first()
                .is_some_and(|owner| config.manifest.settings.owners.contains(owner))
    }))
}

fn default_branch(
    path: &Path,
    remote: &str,
    options: &Options,
    fetch: bool,
) -> Result<Option<String>> {
    if fetch {
        let output = required(
            path,
            &["ls-remote", "--symref", "--", remote, "HEAD"],
            options.timeout,
        )?;
        return Ok(output.lines().find_map(|line| {
            line.strip_prefix("ref: refs/heads/")
                .and_then(|line| line.strip_suffix("\tHEAD"))
                .map(str::to_owned)
        }));
    }
    let prefix = format!("refs/remotes/{remote}/");
    Ok(probe(
        path,
        &["symbolic-ref", "--quiet", &format!("{prefix}HEAD")],
        options.timeout,
    )?
    .and_then(|reference| reference.strip_prefix(&prefix).map(str::to_owned)))
}

enum Return {
    None,
    Review(&'static str),
    Plan(Plan),
}

#[expect(
    clippy::too_many_arguments,
    reason = "explicit evidence inputs keep branch-return safety checks visible"
)]
fn return_plan(
    path: &Path,
    branch: &str,
    head: &str,
    remote: &str,
    upstream: &str,
    upstream_oid: Option<&str>,
    options: &Options,
    fetch: bool,
) -> Result<Return> {
    if upstream.is_empty() {
        return Ok(Return::None);
    }
    // Most checkouts sit on the default branch. When the cached remote HEAD
    // already names the current branch, a second network round trip only to
    // confirm it doubles the cost of status. A stale cache can only hide a
    // possible return, never produce one.
    if default_branch(path, remote, options, false)?.as_deref() == Some(branch) {
        return Ok(Return::None);
    }
    let Some(default) = default_branch(path, remote, options, fetch)? else {
        return Ok(Return::None);
    };
    if default == branch {
        return Ok(Return::None);
    }
    let timeout = options.timeout;
    let Some(target) = probe(
        path,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/remotes/{remote}/{default}"),
        ],
        timeout,
    )?
    else {
        return Ok(Return::None);
    };
    let ancestor = probe(
        path,
        &["merge-base", "--is-ancestor", head, &target],
        timeout,
    )?
    .is_some();
    let pruned = fetch && !upstream.is_empty() && upstream_oid.is_none();
    let patch_merged = if !ancestor && pruned {
        let range = format!("{target}..{head}");
        required(path, &["rev-list", "--merges", &range], timeout)?.is_empty()
            && !required(path, &["cherry", &target, head], timeout)?
                .lines()
                .any(|line| line.starts_with('+'))
    } else {
        false
    };
    if !ancestor && !patch_merged && !merged_pr(path, remote, branch, head, &default, timeout)? {
        return Ok(Return::None);
    }
    let reference = format!("refs/heads/{default}");
    let previous_default = probe(
        path,
        &["rev-parse", "--verify", "--quiet", &reference],
        timeout,
    )?;
    if let Some(oid) = &previous_default
        && probe(
            path,
            &["merge-base", "--is-ancestor", oid, &target],
            timeout,
        )?
        .is_none()
    {
        return Ok(Return::Review("default branch contains local commits"));
    }
    let worktrees = required(path, &["worktree", "list", "--porcelain", "-z"], timeout)?;
    if worktrees
        .split('\0')
        .any(|line| line == format!("branch {reference}"))
    {
        return Ok(Return::Review(
            "default branch is checked out in another worktree",
        ));
    }
    Ok(Return::Plan(Plan {
        head: head.into(),
        target,
        reference: format!("refs/remotes/{remote}/{default}"),
        branch: Some(default),
        previous_default,
    }))
}

fn merged_pr(
    path: &Path,
    remote: &str,
    branch: &str,
    head: &str,
    default: &str,
    timeout: Duration,
) -> Result<bool> {
    let url = required(path, &["remote", "get-url", remote], timeout)?;
    let Ok((host, parts)) = remote_parts(&url) else {
        return Ok(false);
    };
    if host != "github.com" || parts.len() != 2 {
        return Ok(false);
    }
    let repository = parts.join("/");
    let args: Vec<&OsStr> = [
        "pr",
        "view",
        branch,
        "--repo",
        &repository,
        "--json",
        "state,headRefOid,baseRefName",
    ]
    .into_iter()
    .map(OsStr::new)
    .collect();
    let Ok(output) = process::run("gh", &args, path, timeout) else {
        return Ok(false);
    };
    if !output.success {
        return Ok(false);
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&output.stdout) else {
        return Ok(false);
    };
    Ok(
        value.get("state").and_then(serde_json::Value::as_str) == Some("MERGED")
            && value.get("headRefOid").and_then(serde_json::Value::as_str) == Some(head)
            && value.get("baseRefName").and_then(serde_json::Value::as_str) == Some(default),
    )
}

/// Recheck every local precondition without fetching or changing the saved proof.
pub fn validate_plan(report: &Report, options: &Options) -> Result<()> {
    let path = &report.path;
    let timeout = options.timeout;
    let plan = report.plan.as_ref().ok_or("missing plan")?;
    if probe(
        path,
        &["rev-parse", "--verify", "--quiet", &plan.reference],
        timeout,
    )?
    .as_deref()
        != Some(&plan.target)
        || probe(path, &["rev-parse", "--verify", "--quiet", "HEAD"], timeout)?.as_deref()
            != Some(&plan.head)
        || probe(
            path,
            &["symbolic-ref", "--quiet", "--short", "HEAD"],
            timeout,
        )? != report.branch
        || !dirty(&required(
            path,
            &[
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--ignore-submodules=none",
            ],
            timeout,
        )?)?
        .clean()
        || blocked(path, timeout)?.is_some()
        || ignored_collision(path, &plan.target, timeout)?
    {
        return Err("repository changed after inspection".into());
    }
    if let Some(branch) = &plan.branch {
        let reference = format!("refs/heads/{branch}");
        if probe(
            path,
            &["rev-parse", "--verify", "--quiet", &reference],
            timeout,
        )? != plan.previous_default
            || required(path, &["worktree", "list", "--porcelain", "-z"], timeout)?
                .split('\0')
                .any(|line| line == format!("branch {reference}"))
        {
            return Err("default branch changed after inspection".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(state: &str, action: &str, reason: &str, dirty: Dirty, stashes: usize) -> Report {
        let mut report = Report::empty(Path::new("/projects/owner/name"));
        state.clone_into(&mut report.state);
        action.clone_into(&mut report.action);
        reason.clone_into(&mut report.reason);
        report.dirty = dirty;
        report.stashes = stashes;
        report.branch = Some("feature".into());
        report.remote = Some("upstream".into());
        report
    }

    fn untracked() -> Dirty {
        Dirty {
            untracked: 1,
            ..Dirty::default()
        }
    }

    #[test]
    fn local_work_is_separated_from_repositories_that_need_a_decision() {
        let held = report("behind", "review", GENERIC_REASONS[0], untracked(), 0);
        assert!(Group::of(&held) == Group::HeldBack);
        let working = report("synced", "review", GENERIC_REASONS[0], untracked(), 0);
        assert!(Group::of(&working) == Group::Working);
        let stash = report("synced", "review", GENERIC_REASONS[1], Dirty::default(), 1);
        assert!(Group::of(&stash) == Group::Working);
        let diverged = report(
            "diverged",
            "review",
            GENERIC_REASONS[1],
            Dirty::default(),
            0,
        );
        assert!(Group::of(&diverged) == Group::Review);
        // A specific safety reason is never softened into "work in progress".
        let operation = report(
            "synced",
            "review",
            "Git operation in progress",
            untracked(),
            0,
        );
        assert!(Group::of(&operation) == Group::Review);
        let mut failed = held;
        failed.fail("fetch failed");
        assert!(Group::of(&failed) == Group::Failed);
    }

    #[test]
    fn advice_names_the_exact_remote_and_branch() {
        let unpushed = report(
            "no-upstream",
            "review",
            GENERIC_REASONS[1],
            Dirty::default(),
            0,
        );
        assert_eq!(
            hint(&unpushed).as_deref(),
            Some("push it with git push -u upstream feature")
        );
        let specific = report(
            "synced",
            "review",
            "submodules require manual review",
            Dirty::default(),
            0,
        );
        assert_eq!(
            hint(&specific).as_deref(),
            Some("submodules require manual review")
        );
        let remote = report(
            "no-remote",
            "review",
            "no remote configured",
            Dirty::default(),
            0,
        );
        assert!(hint(&remote).is_some_and(|hint| hint.contains("repot publish")));
    }
}
