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
    let reports = collect(config, options)?;
    render(&reports, options.json)
}

pub fn collect(config: &Config, options: &Options) -> Result<Vec<Report>> {
    let repositories = discovery::discover(config)?;
    let chunk_size = repositories
        .len()
        .div_ceil(options.jobs.clamp(1, 32))
        .max(1);
    let mut reports = std::thread::scope(|scope| {
        let handles: Vec<_> = repositories
            .chunks(chunk_size)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|repo| {
                            inspect(&repo.path, config, options, !options.no_fetch).unwrap_or_else(
                                |_| {
                                    let mut report = Report::empty(&repo.path);
                                    report.state = "inspection-failed".into();
                                    report
                                        .fail("could not inspect repository; inspect it with git");
                                    report
                                },
                            )
                        })
                        .collect::<Vec<_>>()
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

pub fn render(reports: &[Report], json: bool) -> Result<u8> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(reports).map_err(|error| error.to_string())?
        );
    } else if std::io::stdout().is_terminal() {
        print!("{}", terminal_report(reports));
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

fn terminal_report(reports: &[Report]) -> String {
    use crossterm::style::{Color, Stylize};

    let color = std::env::var_os("NO_COLOR").is_none()
        && std::env::var_os("TERM").is_none_or(|term| term != "dumb");
    let heading = format!(
        "repot  ·  {} repositories\n\n{:<15} {:<20} REPOSITORY\n",
        reports.len(),
        "ACTION",
        "BRANCH"
    );
    let heading = if color {
        heading.bold().to_string()
    } else {
        heading
    };
    let rows = reports.iter().fold(String::new(), |mut rows, report| {
        let action = if report.applied {
            format!("done: {}", report.action)
        } else {
            report.action.clone()
        };
        let action = format!("{action:<15}");
        let action = if color {
            let tint = if report.failed {
                Color::Red
            } else if matches!(report.action.as_str(), "review" | "push") {
                Color::Yellow
            } else {
                Color::Green
            };
            action.with(tint).to_string()
        } else {
            action
        };
        let branch = terminal_text(report.branch.as_deref().unwrap_or("(detached)"));
        let path = terminal_text(&report.path.to_string_lossy());
        let details = format!(
            "  {} · +{} / -{} commits · staged {} / changed {} / untracked {} · stashes {}\n  {}\n",
            terminal_text(&report.state),
            report.ahead,
            report.behind,
            report.dirty.staged,
            report.dirty.unstaged,
            report.dirty.untracked,
            report.stashes,
            terminal_text(&report.reason)
        );
        let details = if color {
            details.dim().to_string()
        } else {
            details
        };
        let _ = writeln!(rows, "{action} {branch:<20} {path}\n{details}");
        rows
    });
    let applied = reports.iter().filter(|report| report.applied).count();
    let failed = reports.iter().filter(|report| report.failed).count();
    let attention = reports
        .iter()
        .filter(|report| matches!(report.action.as_str(), "review" | "push"))
        .count();
    format!("{heading}{rows}{applied} applied · {attention} need attention · {failed} failed\n")
}

fn terminal_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                character.escape_default().to_string()
            } else {
                character.to_string()
            }
        })
        .collect()
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
