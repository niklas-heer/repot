//! ghq-compatible root output, filtering and shortest unique repository names.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use clap::Args;
use serde::Serialize;

use crate::Result;
use crate::config::{Config, remote_parts};

#[derive(Debug, Args)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent ghq-compatible CLI switches"
)]
pub struct ListOptions {
    /// Show only repositories whose path contains this text.
    pub query: Option<String>,
    /// Print machine-readable results.
    #[arg(long)]
    pub json: bool,
    /// Print absolute paths instead of paths relative to the root.
    #[arg(short = 'p', long)]
    pub full_path: bool,
    /// Match the query against the whole name exactly.
    #[arg(short = 'e', long)]
    pub exact: bool,
    /// Print the shortest unique name for each repository.
    #[arg(long)]
    pub unique: bool,
    /// List bare repositories too.
    #[arg(long)]
    pub bare: bool,
    /// Version control system; only git is supported.
    #[arg(long, hide = true)]
    pub vcs: Option<String>,
}

#[derive(Serialize)]
struct Entry {
    path: PathBuf,
    #[serde(skip)]
    relative: String,
}

pub fn root(config: &Config, all: bool) -> Result<u8> {
    if all {
        for root in config.roots.iter().rev() {
            println!("{}", root.display());
        }
    } else {
        println!("{}", config.primary_root()?.display());
    }
    Ok(0)
}

pub fn list(config: &Config, options: &ListOptions) -> Result<u8> {
    if options
        .vcs
        .as_deref()
        .is_some_and(|vcs| !matches!(vcs, "git" | "github" | "codecommit"))
    {
        return Err(
            "repot supports Git repositories; --vcs accepts git, github or codecommit".into(),
        );
    }
    let query = normalize_query(options.query.as_deref().unwrap_or_default(), options.bare);
    let mut entries = collect(config)?;
    entries.retain(|entry| matches_query(&entry.relative, &query, options.exact));
    if options.unique {
        let mut seen = BTreeSet::new();
        entries.retain(|entry| seen.insert(entry.relative.clone()));
    }
    if options.json {
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        println!(
            "{}",
            serde_json::to_string_pretty(&entries).map_err(|error| error.to_string())?
        );
    } else {
        let mut lines = if options.unique {
            unique_names(&entries)
        } else {
            entries
                .into_iter()
                .map(|entry| {
                    if options.full_path {
                        entry.path.to_string_lossy().into_owned()
                    } else {
                        entry.relative
                    }
                })
                .collect()
        };
        lines.sort();
        for line in lines {
            println!("{line}");
        }
    }
    Ok(0)
}

pub fn matching_paths(config: &Config, query: &str, bare: bool) -> Result<Vec<PathBuf>> {
    let normalized = normalize_query(query, bare);
    Ok(collect(config)?
        .into_iter()
        .filter(|entry| {
            entry.path == Path::new(query) || matches_query(&entry.relative, &normalized, true)
        })
        .map(|entry| entry.path)
        .collect())
}

fn collect(config: &Config) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    let mut seen = BTreeSet::new();
    for root in config.roots.iter().rev() {
        for path in walk(root)? {
            if seen.insert(path.clone()) {
                let relative = path.strip_prefix(root).map_or_else(
                    |_| path.to_string_lossy().into_owned(),
                    |relative| relative.to_string_lossy().into_owned(),
                );
                entries.push(Entry { path, relative });
            }
        }
    }
    for entry in &config.manifest.repo {
        let path = entry.path.as_ref().map_or_else(
            || config.destination(&entry.url),
            |path| config.expand_path(path),
        )?;
        if !crate::discovery::private_path(&path) && is_repository(&path) {
            let path = path
                .canonicalize()
                .map_err(|error| format!("resolve checkout: {error}"))?;
            if crate::discovery::private_path(&path) {
                continue;
            }
            if seen.insert(path.clone()) {
                entries.push(Entry {
                    relative: path.to_string_lossy().into_owned(),
                    path,
                });
            }
        }
    }
    Ok(entries)
}

fn walk(root: &Path) -> Result<Vec<PathBuf>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        if crate::discovery::private_path(&path) {
            continue;
        }
        let metadata =
            fs::symlink_metadata(&path).map_err(|error| format!("inspect directory: {error}"))?;
        if !metadata.is_dir() || metadata.is_symlink() {
            continue;
        }
        if is_repository(&path) {
            let path = path
                .canonicalize()
                .map_err(|error| format!("resolve checkout: {error}"))?;
            if !crate::discovery::private_path(&path) {
                paths.push(path);
            }
            continue;
        }
        let children = match fs::read_dir(&path) {
            Ok(children) => children,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => continue,
            Err(error) => return Err(format!("read directory: {error}")),
        };
        for child in children {
            let child = child.map_err(|error| format!("read directory entry: {error}"))?;
            if child
                .file_type()
                .map_err(|error| format!("inspect directory entry: {error}"))?
                .is_dir()
            {
                pending.push(child.path());
            }
        }
    }
    paths.sort();
    Ok(paths)
}

fn is_repository(path: &Path) -> bool {
    let dotgit = path.join(".git");
    dotgit.is_file()
        || dotgit.is_dir()
        || (path.extension().is_some_and(|extension| extension == "git")
            && path.join("HEAD").is_file()
            && path.join("objects").is_dir()
            && path.join("refs").is_dir())
}

fn normalize_query(query: &str, bare: bool) -> String {
    if (query.contains("://") || query.contains(':'))
        && let Ok((host, parts)) = remote_parts(query)
    {
        let relative = std::iter::once(host)
            .chain(parts)
            .collect::<Vec<_>>()
            .join("/");
        return if bare {
            format!("{relative}.git")
        } else {
            relative
        };
    }
    query.into()
}

fn matches_query(relative: &str, query: &str, exact: bool) -> bool {
    if query.is_empty() {
        return true;
    }
    if exact {
        return subpaths(relative).contains(&query);
    }
    let (host, query) = query
        .split_once('/')
        .filter(|(host, _)| host.contains('.'))
        .map_or((None, query), |(host, query)| (Some(host), query));
    let (repository_host, rest) = relative.split_once('/').unwrap_or((relative, relative));
    if host.is_some_and(|host| host != repository_host) {
        return false;
    }
    if query.to_lowercase() == query {
        rest.to_lowercase().contains(query)
    } else {
        rest.contains(query)
    }
}

fn subpaths(path: &str) -> Vec<&str> {
    let mut remaining = path;
    let mut paths = vec![remaining];
    while let Some((_, tail)) = remaining.split_once('/') {
        paths.push(tail);
        remaining = tail;
    }
    paths.reverse();
    paths
}

fn unique_names(entries: &[Entry]) -> Vec<String> {
    let mut counts = BTreeMap::<&str, usize>::new();
    for entry in entries {
        for path in subpaths(&entry.relative) {
            let count = counts.entry(path).or_default();
            *count = count.saturating_add(1);
        }
    }
    entries
        .iter()
        .filter_map(|entry| {
            subpaths(&entry.relative)
                .into_iter()
                .find(|path| counts.get(path) == Some(&1))
                .map(str::to_owned)
        })
        .collect()
}
