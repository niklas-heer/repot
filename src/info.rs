//! A local snapshot of one repository for `repot info` and the picker preview.
//! Everything here is read without touching the network or writing to Git.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::process;
use crate::ui::{self, Paint};
use crate::visits::{self, Visits};

#[derive(Clone, Debug, Serialize)]
pub struct Commit {
    pub hash: String,
    pub subject: String,
    pub author: String,
    pub time: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Info {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    /// Counts against the cached upstream; run `repot status` to refresh.
    pub ahead: usize,
    pub behind: usize,
    pub staged: usize,
    pub modified: usize,
    pub untracked: usize,
    pub stashes: usize,
    pub remote: Option<String>,
    pub commits: Vec<Commit>,
    pub visits: usize,
    pub last_visit: Option<u64>,
    #[serde(skip)]
    pub unreadable: bool,
}

fn git(path: &Path, args: &[&str]) -> Option<String> {
    process::git_optional(path, args).ok().flatten()
}

/// Drop credentials from HTTP remotes, where tokens are often embedded as the
/// user name. SSH user names such as `git@` are not secrets and stay.
pub fn public_url(url: &str) -> String {
    if let Some((scheme, rest)) = url
        .split_once("://")
        .filter(|(scheme, _)| matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https"))
    {
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        return ui::visible(&format!("{scheme}://{host}/{path}"));
    }
    ui::visible(url)
}

pub fn gather(path: &Path, visits: Option<&Visits>) -> Info {
    let mut info = Info {
        path: path.to_path_buf(),
        visits: visits.map_or(0, |visits| visits.count),
        last_visit: visits.and_then(|visits| visits.last),
        ..Info::default()
    };
    let Some(status) = git(
        path,
        &[
            "status",
            "--porcelain=v2",
            "--branch",
            "--show-stash",
            "--untracked-files=normal",
            "-z",
        ],
    ) else {
        info.unreadable = true;
        return info;
    };
    let mut records = status.split('\0');
    while let Some(record) = records.next() {
        if let Some(header) = record.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.head" if value != "(detached)" => info.branch = Some(value.to_owned()),
                "branch.upstream" => info.upstream = Some(value.to_owned()),
                "branch.ab" => {
                    let mut counts = value.split(' ');
                    let mut count = |prefix: char| {
                        counts
                            .next()
                            .and_then(|count| count.strip_prefix(prefix))
                            .and_then(|count| count.parse().ok())
                            .unwrap_or_default()
                    };
                    info.ahead = count('+');
                    info.behind = count('-');
                }
                "stash" => info.stashes = value.parse().unwrap_or_default(),
                _ => {}
            }
            continue;
        }
        let mut fields = record.splitn(3, ' ');
        match (fields.next(), fields.next()) {
            (Some(kind @ ("1" | "2" | "u")), Some(xy)) => {
                let mut flags = xy.chars();
                if flags.next().is_some_and(|flag| flag != '.') {
                    info.staged = info.staged.saturating_add(1);
                }
                if flags.next().is_some_and(|flag| flag != '.') {
                    info.modified = info.modified.saturating_add(1);
                }
                if kind == "2" {
                    // Renames carry their original path as the next record.
                    records.next();
                }
            }
            (Some("?"), _) => info.untracked = info.untracked.saturating_add(1),
            _ => {}
        }
    }
    let remote_name = info
        .upstream
        .as_deref()
        .and_then(|upstream| upstream.split_once('/'))
        .map_or("origin", |(remote, _)| remote)
        .to_owned();
    info.remote = git(path, &["remote", "get-url", "--", &remote_name])
        .or_else(|| {
            git(path, &["remote"])
                .and_then(|remotes| remotes.lines().next().map(str::to_owned))
                .and_then(|remote| git(path, &["remote", "get-url", "--", &remote]))
        })
        .map(|url| public_url(&url));
    info.commits = git(
        path,
        &[
            "log",
            "-5",
            "--no-show-signature",
            "--format=%h%x1f%s%x1f%an%x1f%ct%x1e",
        ],
    )
    .map(|log| {
        log.split('\u{1e}')
            .filter_map(|entry| {
                let mut fields = entry.trim_start_matches('\n').split('\u{1f}');
                Some(Commit {
                    hash: fields.next()?.to_owned(),
                    subject: ui::visible(fields.next()?),
                    author: ui::visible(fields.next()?),
                    time: fields.next()?.trim().parse().ok()?,
                })
            })
            .collect()
    })
    .unwrap_or_default();
    info
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// Staged, modified, untracked and stashed work in one line, if any.
pub fn changes(info: &Info) -> Option<String> {
    let parts: Vec<String> = [
        (info.staged, "staged"),
        (info.modified, "modified"),
        (info.untracked, "untracked"),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, label)| format!("{count} {label}"))
    .chain((info.stashes > 0).then(|| plural(info.stashes, "stash", "stashes")))
    .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

pub fn branch_line(info: &Info) -> String {
    let branch = info
        .branch
        .as_deref()
        .map_or_else(|| "detached HEAD".to_owned(), ui::visible);
    info.upstream.as_deref().map_or_else(
        || format!("{branch}  (no upstream)"),
        |upstream| {
            let mut line = format!("{branch} → {}", ui::visible(upstream));
            if info.ahead > 0 {
                let _ = write!(line, "  ↑{}", info.ahead);
            }
            if info.behind > 0 {
                let _ = write!(line, "  ↓{}", info.behind);
            }
            if info.ahead == 0 && info.behind == 0 {
                line.push_str("  in sync");
            }
            line
        },
    )
}

pub fn visit_line(info: &Info) -> String {
    info.last_visit.map_or_else(
        || "not visited yet".into(),
        |last| {
            format!(
                "{}, last {}",
                plural(info.visits, "visit", "visits"),
                visits::ago(last)
            )
        },
    )
}

/// The `repot info` report.
pub fn render(info: &Info, roots: &[PathBuf]) -> String {
    let paint = Paint::stdout();
    let label = |text: &str| paint.paint(ui::DIM, format!("{text:<9}"));
    let mut out = format!(
        "\n {}\n",
        paint.paint(ui::BOLD, ui::repository_name(&info.path, roots))
    );
    let mut row = |name: &str, value: String| {
        let _ = writeln!(out, "   {} {value}", label(name));
    };
    row("path", ui::repository_name(&info.path, &[]));
    if info.unreadable {
        row(
            "status",
            paint.paint(ui::BAD, "Git could not read this checkout"),
        );
        return out;
    }
    row(
        "remote",
        info.remote
            .clone()
            .unwrap_or_else(|| paint.paint(ui::WARN, "none")),
    );
    row("branch", branch_line(info));
    row(
        "changes",
        changes(info).unwrap_or_else(|| paint.paint(ui::GOOD, "clean")),
    );
    row("visits", visit_line(info));
    for (position, commit) in info.commits.iter().enumerate() {
        row(
            if position == 0 { "recent" } else { "" },
            format!(
                "{}  {}  {}",
                paint.paint(ui::INFO, &commit.hash),
                paint.paint(ui::DIM, format!("{:<11}", visits::ago(commit.time))),
                commit.subject
            ),
        );
    }
    if info.upstream.is_some() {
        let _ = write!(
            out,
            "\n {}\n",
            paint.paint(
                ui::DIM,
                "ahead/behind use cached remote refs; repot status fetches fresh ones"
            )
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_never_reach_the_output() {
        assert_eq!(
            public_url("https://user:token@example.com/owner/repo.git"),
            "https://example.com/owner/repo.git"
        );
        assert_eq!(
            public_url("https://ghp_token@example.com/owner/repo"),
            "https://example.com/owner/repo"
        );
        assert_eq!(
            public_url("ssh://git@example.com/owner/repo"),
            "ssh://git@example.com/owner/repo"
        );
        assert_eq!(
            public_url("git@github.com:owner/repo.git"),
            "git@github.com:owner/repo.git"
        );
        assert_eq!(
            public_url("/local/path\u{1b}[31m"),
            "/local/path\\u{1b}[31m"
        );
    }
}
