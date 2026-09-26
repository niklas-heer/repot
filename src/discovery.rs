//! Deterministic checkout discovery without following directory symlinks.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::Result;
use crate::config::Config;

#[derive(Clone, Debug, Serialize)]
pub struct Repository {
    pub path: PathBuf,
}

pub fn discover(config: &Config) -> Result<Vec<Repository>> {
    let mut paths = BTreeSet::new();
    for root in &config.roots {
        walk(root, false, &mut paths)?;
    }
    for entry in &config.manifest.repo {
        let path = match &entry.path {
            Some(path) => config.expand_path(path)?,
            None => config.destination(&entry.url)?,
        };
        if is_repository(&path) {
            paths.insert(canonical(&path)?);
        }
    }
    Ok(repositories(paths))
}

fn repositories(paths: BTreeSet<PathBuf>) -> Vec<Repository> {
    paths.into_iter().map(|path| Repository { path }).collect()
}

fn canonical(path: &Path) -> Result<PathBuf> {
    path.canonicalize()
        .map_err(|error| format!("resolve {}: {error}", path.display()))
}

fn is_repository(path: &Path) -> bool {
    let git = path.join(".git");
    git.is_dir() || git.is_file()
}

fn walk(start: &Path, exclude: bool, paths: &mut BTreeSet<PathBuf>) -> Result<()> {
    if !start.exists() {
        return Ok(());
    }
    let mut pending = vec![start.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("inspect {}: {error}", path.display()))?;
        if metadata.is_symlink() || !metadata.is_dir() {
            continue;
        }
        if is_repository(&path) {
            paths.insert(canonical(&path)?);
            continue;
        }
        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(error) if exclude && error.kind() == std::io::ErrorKind::PermissionDenied => {
                continue;
            }
            Err(error) => return Err(format!("read directory {}: {error}", path.display())),
        };
        for entry in entries {
            let entry = entry.map_err(|error| format!("read directory entry: {error}"))?;
            if exclude && excluded(&entry.file_name().to_string_lossy()) {
                continue;
            }
            let kind = entry
                .file_type()
                .map_err(|error| format!("inspect directory entry: {error}"))?;
            if kind.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    Ok(())
}

fn excluded(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".cache"
            | ".Trash"
            | "node_modules"
            | "target"
            | "vendor"
            | ".venv"
            | "venv"
            | "Library"
    )
}
