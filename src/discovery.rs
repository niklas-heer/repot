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
        if !private_path(&path) && is_repository(&path) {
            let path = canonical(&path)?;
            if !private_path(&path) {
                paths.insert(path);
            }
        }
    }
    Ok(repositories(paths))
}

pub fn find(path: &Path) -> Result<Vec<Repository>> {
    if !path.is_dir() {
        return Err(format!(
            "search path is not a directory: {}",
            path.display()
        ));
    }
    let mut paths = BTreeSet::new();
    walk(path, true, &mut paths)?;
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

/// Private staging and archived repositories are never active checkouts.
pub fn private_path(path: &Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        name == ".repot-trash"
            || [
                ".repot-clone-",
                ".repot-new-",
                ".repot-create-",
                ".repot-restore-",
            ]
            .iter()
            .any(|prefix| name.starts_with(prefix))
    })
}

pub fn validate_public_path(path: &Path) -> Result<()> {
    let resolved = path
        .ancestors()
        .find_map(|ancestor| ancestor.canonicalize().ok());
    if private_path(path) || resolved.as_deref().is_some_and(private_path) {
        return Err("repository paths cannot contain reserved repot staging or archive components (.repot-clone-*, .repot-new-*, .repot-create-*, .repot-restore-* or .repot-trash)".into());
    }
    Ok(())
}

fn walk(start: &Path, exclude: bool, paths: &mut BTreeSet<PathBuf>) -> Result<()> {
    if !start.exists() {
        return Ok(());
    }
    let mut pending = vec![start.to_path_buf()];
    while let Some(path) = pending.pop() {
        if private_path(&path) {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("inspect {}: {error}", path.display()))?;
        if metadata.is_symlink() || !metadata.is_dir() {
            continue;
        }
        if is_repository(&path) {
            let path = canonical(&path)?;
            if !private_path(&path) {
                paths.insert(path);
            }
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
