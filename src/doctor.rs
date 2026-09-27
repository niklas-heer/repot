//! Find checkouts that drifted out of place: duplicates of the same repository,
//! repositories renamed or archived on GitHub, and checkouts whose folder no
//! longer matches their remote. Report-only: every finding comes with the
//! command that fixes it, and `repot rm` keeps removed checkouts recoverable.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::config::{Config, remote_parts};
use crate::discovery::{self, Repository};
use crate::work::parallel;
use crate::{Result, process, ui};

#[derive(Debug, clap::Args)]
pub struct Options {
    /// Print machine-readable results.
    #[arg(long)]
    pub json: bool,
    /// Skip asking GitHub (through gh) about renames, transfers and archives.
    #[arg(long)]
    pub offline: bool,
    /// Maximum concurrent repositories.
    #[arg(long, default_value_t = 16, value_parser = clap::value_parser!(u8).range(1..=32))]
    pub jobs: u8,
    /// Timeout for each Git or gh call, in seconds.
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=3600))]
    pub timeout: u64,
}

/// What GitHub says about a remote.
enum GitHub {
    /// gh was unavailable or the request failed; say nothing.
    Unknown,
    /// Not found, or not visible to the signed-in account.
    Missing,
    Found {
        full_name: String,
        archived: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Duplicate,
    Renamed,
    Misplaced,
    Missing,
    Archived,
}

#[derive(Debug, Serialize)]
pub struct Finding {
    pub kind: Kind,
    pub path: PathBuf,
    /// The checkout or remote this one relates to, if any.
    pub related: Option<String>,
    pub detail: String,
    /// A command that resolves the finding.
    pub fix: String,
    /// Work that exists only in this checkout, worth a look before removing it.
    pub local_work: Option<String>,
}

/// What one checkout claims to be, from its remote.
struct Identity {
    path: PathBuf,
    url: String,
    host: String,
    parts: Vec<String>,
}

impl Identity {
    fn key(&self) -> String {
        key(&self.host, &self.parts)
    }
}

/// Forges treat names case-insensitively; so does duplicate detection.
fn key(host: &str, parts: &[String]) -> String {
    format!("{host}/{}", parts.join("/")).to_lowercase()
}

fn identify(path: &Path) -> Option<Identity> {
    let remotes = process::git_optional(path, &["remote"]).ok()??;
    let remote = if remotes.lines().any(|remote| remote == "origin") {
        "origin"
    } else {
        remotes.lines().next()?
    };
    let url = process::git_optional(path, &["remote", "get-url", "--", remote]).ok()??;
    let (host, parts) = remote_parts(&url).ok()?;
    Some(Identity {
        path: path.to_path_buf(),
        url,
        host,
        parts,
    })
}

/// A path to paste into a shell: `~/…` when that is unambiguous, otherwise the
/// quoted absolute path.
fn quoted(path: &Path) -> String {
    let safe = |text: &str| {
        text.chars()
            .all(|character| character.is_ascii_alphanumeric() || "/._-~+".contains(character))
    };
    let short = ui::repository_name(path, &[]);
    if safe(&short) {
        return short;
    }
    let text = ui::visible(&path.to_string_lossy());
    if safe(&text) {
        text
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

/// Uncommitted files, stashes and commits that no remote has.
pub fn local_work(path: &Path) -> Option<String> {
    let count = |args: &[&str]| {
        process::git_optional(path, args)
            .ok()
            .flatten()
            .map_or(0, |output| {
                output.lines().filter(|line| !line.is_empty()).count()
            })
    };
    let changed = count(&["status", "--porcelain", "--untracked-files=normal"]);
    let stashes = count(&["stash", "list", "--format=%H"]);
    let unpushed = count(&["rev-list", "HEAD", "--not", "--remotes"]);
    // repot rm refuses checkouts with linked worktrees; say so up front.
    let worktrees = process::git_optional(path, &["worktree", "list", "--porcelain"])
        .ok()
        .flatten()
        .map_or(0, |list| {
            list.lines()
                .filter(|line| line.starts_with("worktree "))
                .count()
                .saturating_sub(1)
        });
    let parts: Vec<String> = [
        (changed, "changed file", "changed files"),
        (stashes, "stash", "stashes"),
        (unpushed, "unpushed commit", "unpushed commits"),
        (worktrees, "linked worktree", "linked worktrees"),
    ]
    .into_iter()
    .filter(|(count, _, _)| *count > 0)
    .map(|(count, one, many)| format!("{count} {}", if count == 1 { one } else { many }))
    .collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

/// Where the tree says a checkout of this remote belongs, relative to a root.
fn misplaced(identity: &Identity, roots: &[PathBuf]) -> Option<PathBuf> {
    for root in roots {
        let root = root.canonicalize().unwrap_or_else(|_| root.clone());
        let Ok(relative) = identity.path.strip_prefix(&root) else {
            continue;
        };
        if relative.starts_with("local") {
            return None;
        }
        let expected: PathBuf = std::iter::once(identity.host.as_str())
            .chain(identity.parts.iter().map(String::as_str))
            .collect();
        let matches = relative
            .to_string_lossy()
            .eq_ignore_ascii_case(&expected.to_string_lossy());
        return (!matches).then(|| root.join(expected));
    }
    None
}

/// Canonical `owner/name` and archive state from GitHub, following renames and
/// transfers.
fn github(identity: &Identity, timeout: Duration) -> GitHub {
    let [owner, name] = identity.parts.as_slice() else {
        return GitHub::Unknown;
    };
    let endpoint = format!("repos/{owner}/{name}");
    let args: Vec<&OsStr> = [
        "api",
        &endpoint,
        "--jq",
        r#".full_name + "\t" + (.archived | tostring)"#,
    ]
    .into_iter()
    .map(OsStr::new)
    .collect();
    let Ok(output) = process::run("gh", &args, &identity.path, timeout) else {
        return GitHub::Unknown;
    };
    if !output.success {
        return if output.stdout.contains("\"status\":\"404\"") {
            GitHub::Missing
        } else {
            GitHub::Unknown
        };
    }
    output
        .stdout
        .trim()
        .split_once('\t')
        .map_or(GitHub::Unknown, |(full_name, archived)| GitHub::Found {
            full_name: full_name.to_owned(),
            archived: archived == "true",
        })
}

fn renamed_url(url: &str, full_name: &str) -> String {
    if url.starts_with("git@") || url.starts_with("ssh://") {
        format!("git@github.com:{full_name}.git")
    } else {
        format!("https://github.com/{full_name}.git")
    }
}

pub fn run(config: &Config, options: &Options) -> Result<u8> {
    let repositories: Vec<Repository> = discovery::discover(config)?;
    let jobs = usize::from(options.jobs);
    let timeout = Duration::from_secs(options.timeout);
    let progress = (!options.json).then(|| ui::Progress::start("Checking", repositories.len()));
    let identities: Vec<Identity> = parallel(&repositories, jobs, progress.as_ref(), |repo| {
        identify(&repo.path)
    })?
    .into_iter()
    .flatten()
    .collect();
    let online = !options.offline
        && process::run("gh", &[OsStr::new("--version")], Path::new("/"), timeout)
            .is_ok_and(|output| output.success);
    let answers = if online {
        let hosted: Vec<&Identity> = identities
            .iter()
            .filter(|identity| identity.host == "github.com")
            .collect();
        if let Some(progress) = &progress {
            progress.phase("Asking GitHub", hosted.len());
        }
        let answers = parallel(&hosted, jobs.min(8), progress.as_ref(), |identity| {
            github(identity, timeout)
        })?;
        hosted
            .into_iter()
            .map(|identity| identity.path.clone())
            .zip(answers)
            .collect()
    } else {
        BTreeMap::new()
    };
    drop(progress);
    let findings = findings(config, &identities, &answers);
    render(&findings, identities.len(), online, options.json)?;
    Ok(if findings.is_empty() { 0 } else { 3 })
}

type Answers = BTreeMap<PathBuf, GitHub>;

/// What a checkout really is: GitHub's current name when known.
fn actual(identity: &Identity, answers: &Answers) -> String {
    match answers.get(&identity.path) {
        Some(GitHub::Found { full_name, .. }) => format!("github.com/{full_name}").to_lowercase(),
        _ => identity.key(),
    }
}

/// Checkouts of the same repository under different paths, keeping the one
/// that already sits where its current name belongs.
fn duplicates(config: &Config, identities: &[Identity], answers: &Answers) -> Vec<Finding> {
    let mut findings = Vec::new();
    let actual = |identity: &Identity| actual(identity, answers);
    let mut groups: BTreeMap<String, Vec<&Identity>> = BTreeMap::new();
    for identity in identities {
        groups.entry(actual(identity)).or_default().push(identity);
    }
    for members in groups.values().filter(|members| members.len() > 1) {
        let keep = members
            .iter()
            .find(|identity| {
                identity.key() == actual(identity) && misplaced(identity, &config.roots).is_none()
            })
            .or_else(|| members.first())
            .copied();
        for identity in members {
            let Some(keep) = keep.filter(|keep| keep.path != identity.path) else {
                continue;
            };
            findings.push(Finding {
                kind: Kind::Duplicate,
                path: identity.path.clone(),
                related: Some(ui::repository_name(&keep.path, &config.roots)),
                detail: format!(
                    "same repository as {}",
                    ui::repository_name(&keep.path, &config.roots)
                ),
                fix: format!("repot rm {}", quoted(&identity.path)),
                local_work: local_work(&identity.path),
            });
        }
    }
    findings
}

fn findings(config: &Config, identities: &[Identity], answers: &Answers) -> Vec<Finding> {
    let mut findings = duplicates(config, identities, answers);
    let duplicated: Vec<PathBuf> = findings
        .iter()
        .map(|finding| finding.path.clone())
        .collect();
    for identity in identities {
        if duplicated.contains(&identity.path) {
            continue;
        }
        match answers.get(&identity.path) {
            Some(GitHub::Found {
                full_name,
                archived,
            }) => {
                let old = identity.parts.join("/");
                if !full_name.eq_ignore_ascii_case(&old) {
                    let url = renamed_url(&identity.url, full_name);
                    findings.push(Finding {
                        kind: Kind::Renamed,
                        path: identity.path.clone(),
                        related: Some(full_name.clone()),
                        detail: format!("now {full_name} on GitHub (was {old})"),
                        fix: format!(
                            "git -C {path} remote set-url origin {url} && repot adopt {path}",
                            path = quoted(&identity.path)
                        ),
                        local_work: None,
                    });
                    continue;
                }
                if *archived {
                    findings.push(Finding {
                        kind: Kind::Archived,
                        path: identity.path.clone(),
                        related: None,
                        detail: "archived on GitHub; it will not change anymore".into(),
                        fix: format!("repot rm {}", quoted(&identity.path)),
                        local_work: local_work(&identity.path),
                    });
                }
            }
            Some(GitHub::Missing) => findings.push(Finding {
                kind: Kind::Missing,
                path: identity.path.clone(),
                related: None,
                detail: format!(
                    "{} is not on GitHub, or not visible to you",
                    identity.parts.join("/")
                ),
                fix: format!("repot info {}", quoted(&identity.path)),
                local_work: None,
            }),
            _ => {}
        }
        if let Some(expected) = misplaced(identity, &config.roots) {
            findings.push(Finding {
                kind: Kind::Misplaced,
                path: identity.path.clone(),
                related: Some(ui::repository_name(&expected, &config.roots)),
                detail: format!(
                    "its remote belongs at {}",
                    ui::repository_name(&expected, &config.roots)
                ),
                fix: format!("repot adopt {}", quoted(&identity.path)),
                local_work: None,
            });
        }
    }
    findings.sort_by(|left, right| (left.kind, &left.path).cmp(&(right.kind, &right.path)));
    findings
}

fn render(findings: &[Finding], checked: usize, online: bool, json: bool) -> Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(findings).map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    let paint = ui::Paint::stdout();
    let mut out = String::from("\n");
    let mut kinds: Vec<Kind> = findings.iter().map(|finding| finding.kind).collect();
    kinds.dedup();
    for kind in kinds {
        let (symbol, title, style) = match kind {
            Kind::Duplicate => ("⧉", "Duplicates", ui::WARN),
            Kind::Renamed => ("↪", "Renamed on GitHub", ui::WARN),
            Kind::Misplaced => ("⇄", "Not where their remote belongs", ui::WARN),
            Kind::Missing => ("?", "Not found on GitHub", ui::WARN),
            Kind::Archived => ("▣", "Archived on GitHub", ui::INFO),
        };
        let members: Vec<&Finding> = findings
            .iter()
            .filter(|finding| finding.kind == kind)
            .collect();
        let _ = writeln!(
            out,
            " {} {}  {}",
            paint.paint(style.bold(), symbol),
            paint.paint(ui::BOLD, title),
            paint.paint(ui::DIM, members.len())
        );
        for finding in members {
            let _ = writeln!(
                out,
                "   {}  {}",
                paint.paint(ui::BOLD, ui::repository_name(&finding.path, &[])),
                paint.paint(ui::DIM, &finding.detail),
            );
            if let Some(work) = &finding.local_work {
                let _ = writeln!(
                    out,
                    "     {} {}",
                    paint.paint(ui::WARN, "!"),
                    paint.paint(
                        ui::WARN,
                        format!("only here: {work}; review before removing")
                    )
                );
            }
            let _ = writeln!(out, "     {} {}", paint.paint(ui::INFO, "→"), finding.fix);
        }
        out.push('\n');
    }
    if findings.is_empty() {
        let _ = writeln!(
            out,
            " {} {}",
            paint.paint(ui::GOOD.bold(), "✓"),
            paint.paint(ui::BOLD, "Everything is where it belongs")
        );
    }
    let _ = writeln!(
        out,
        " {}",
        paint.paint(
            ui::DIM,
            format!(
                "{checked} {} checked{}",
                if checked == 1 {
                    "repository"
                } else {
                    "repositories"
                },
                if online {
                    " against GitHub"
                } else {
                    "; renames need gh, see repot doctor --help"
                }
            )
        )
    );
    if findings
        .iter()
        .any(|finding| finding.fix.starts_with("repot rm"))
    {
        let _ = writeln!(
            out,
            " {}",
            paint.paint(
                ui::DIM,
                "repot rm archives the checkout with all its work; repot trash restores it"
            )
        );
    }
    print!("{out}");
    Ok(())
}
