//! Reversible repository removal and explicit recovery from a local archive.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::{Result, ghq_listing, lifecycle, manifest, navigation, process};

#[derive(Debug, Args)]
pub struct RemoveOptions {
    /// Repository to archive: a unique name, owner/name or absolute path.
    pub query: String,
    /// Look for a bare repository.
    #[arg(long)]
    pub bare: bool,
    /// Show what would be archived without moving anything.
    #[arg(long)]
    pub dry_run: bool,
    /// Print machine-readable results.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct MigrateOptions {
    /// Checkout to move into the tree.
    pub path: PathBuf,
    /// Compatibility flag: repot migration is already noninteractive.
    #[arg(short = 'y', long, visible_alias = "y")]
    pub yes: bool,
    /// Show where the checkout would go without moving it.
    #[arg(long)]
    pub dry_run: bool,
    /// Print machine-readable results.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum TrashCommand {
    /// Show recoverable checkouts archived by repot rm.
    List {
        /// Print machine-readable results.
        #[arg(long)]
        json: bool,
    },
    /// Restore an archived checkout to its original vacant location.
    Restore {
        /// Archive ID shown by `repot trash list`.
        id: String,
        /// Check the destination without restoring anything.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    version: u8,
    original_path: PathBuf,
    bare: bool,
}

#[derive(Serialize)]
struct Report {
    id: String,
    path: PathBuf,
    action: &'static str,
}

pub fn migrate(config: &Config, options: &MigrateOptions) -> Result<u8> {
    manifest::adopt(config, &options.path, false, options.dry_run, options.json)
}

pub fn remove(config: &Config, options: &RemoveOptions) -> Result<u8> {
    let matches = ghq_listing::matching_paths(config, &options.query, options.bare)?;
    let source = match matches.as_slice() {
        [source] => source,
        [] => return Err("no repository matches; use repot list to choose one".into()),
        _ => return Err("multiple repositories match; supply an absolute checkout path".into()),
    };
    let bare = process::git(source, &["rev-parse", "--is-bare-repository"])? == "true";
    let root = config
        .roots
        .iter()
        .rev()
        .find(|root| source.starts_with(root))
        .map_or_else(|| config.primary_root(), |root| Ok(root.as_path()))?;
    let archive_root = root.join(".repot-trash");
    validate_archive_root(&archive_root)?;
    preflight(source, &archive_root.join("pending/repo"), bare)?;
    if options.dry_run {
        return render(
            &[Report {
                id: String::new(),
                path: source.clone(),
                action: "would-archive",
            }],
            options.json,
        );
    }
    fs::create_dir_all(&archive_root)
        .map_err(|error| format!("create archive directory: {error}"))?;
    let directory = tempfile::Builder::new()
        .prefix("repo-")
        .tempdir_in(&archive_root)
        .map_err(|error| format!("prepare archive: {error}"))?;
    let metadata = Metadata {
        version: 1,
        original_path: source.clone(),
        bare,
    };
    let bytes = serde_json::to_vec_pretty(&metadata).map_err(|error| error.to_string())?;
    let mut metadata_file = fs::File::create_new(directory.path().join("metadata.json"))
        .map_err(|error| format!("create archive metadata: {error}"))?;
    metadata_file
        .write_all(&bytes)
        .map_err(|error| format!("write archive metadata: {error}"))?;
    metadata_file
        .sync_all()
        .map_err(|error| format!("sync archive metadata: {error}"))?;
    // Disarm automatic recursive cleanup before the directory can contain user work.
    let directory = directory.keep();
    let target = directory.join("repo");
    if let Err(error) = move_repository(source, &target, bare) {
        let _ = fs::remove_file(directory.join("metadata.json"));
        let _ = fs::remove_dir(&directory);
        return Err(error);
    }
    let id = directory
        .file_name()
        .ok_or("archive has no ID")?
        .to_string_lossy()
        .into_owned();
    crate::ui::done(&format!(
        "archived at {}; restore with repot trash restore {id}",
        target.display()
    ));
    render(
        &[Report {
            id,
            path: source.clone(),
            action: "archived",
        }],
        options.json,
    )
}

pub fn trash(config: &Config, command: &TrashCommand) -> Result<u8> {
    match command {
        TrashCommand::List { json } => {
            let mut reports = Vec::new();
            for root in config.roots.iter().rev() {
                let archive_root = root.join(".repot-trash");
                validate_archive_root(&archive_root)?;
                if !archive_root.exists() {
                    continue;
                }
                for entry in
                    fs::read_dir(&archive_root).map_err(|error| format!("read archive: {error}"))?
                {
                    let entry = entry.map_err(|error| format!("read archive entry: {error}"))?;
                    if !entry
                        .file_type()
                        .map_err(|error| error.to_string())?
                        .is_dir()
                    {
                        continue;
                    }
                    if !entry.path().join("repo").exists() {
                        continue;
                    }
                    let metadata = read_metadata(&entry.path())?;
                    reports.push(Report {
                        id: entry.file_name().to_string_lossy().into_owned(),
                        path: metadata.original_path,
                        action: "archived",
                    });
                }
            }
            reports.sort_by(|left, right| left.id.cmp(&right.id));
            render(&reports, *json)
        }
        TrashCommand::Restore { id, dry_run } => restore(config, id, *dry_run),
    }
}

fn restore(config: &Config, id: &str, dry_run: bool) -> Result<u8> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
    {
        return Err("invalid archive ID".into());
    }
    let mut matches = Vec::new();
    for root in config.roots.iter().rev() {
        let archive_root = root.join(".repot-trash");
        validate_archive_root(&archive_root)?;
        let path = archive_root.join(id);
        if let Ok(metadata) = fs::symlink_metadata(&path) {
            if metadata.is_symlink() || !metadata.is_dir() {
                return Err("archive entry is not a real directory".into());
            }
            matches.push(path);
        }
    }
    let directory = match matches.as_slice() {
        [directory] => directory,
        [] => return Err("archive ID was not found".into()),
        _ => return Err("archive ID exists under multiple roots; narrow GHQ_ROOT".into()),
    };
    let metadata = read_metadata(directory)?;
    let source = directory.join("repo");
    preflight(&source, &metadata.original_path, metadata.bare)?;
    if dry_run {
        println!("would restore {id} to {}", metadata.original_path.display());
        return Ok(0);
    }
    move_repository(&source, &metadata.original_path, metadata.bare)?;
    fs::remove_file(directory.join("metadata.json")).map_err(|error| {
        format!(
            "checkout restored to {} but archive metadata cleanup failed: {error}",
            metadata.original_path.display()
        )
    })?;
    fs::remove_dir(directory).map_err(|error| {
        format!("checkout restored but archive directory cleanup failed: {error}")
    })?;
    crate::ui::done(&format!("restored to {}", metadata.original_path.display()));
    navigation::handoff(&metadata.original_path)?;
    Ok(0)
}

fn read_metadata(directory: &Path) -> Result<Metadata> {
    let path = directory.join("metadata.json");
    let kind = fs::symlink_metadata(&path)
        .map_err(|error| format!("inspect archive metadata: {error}"))?;
    if !kind.is_file() || kind.is_symlink() {
        return Err("archive metadata must be a regular file".into());
    }
    let source = fs::symlink_metadata(directory.join("repo"))
        .map_err(|error| format!("inspect archived checkout: {error}"))?;
    if !source.is_dir() || source.is_symlink() {
        return Err("archived checkout must be a real directory".into());
    }
    let metadata: Metadata = serde_json::from_slice(
        &fs::read(path).map_err(|error| format!("read archive metadata: {error}"))?,
    )
    .map_err(|_| "archive metadata is invalid".to_owned())?;
    if metadata.version != 1 || !metadata.original_path.is_absolute() {
        return Err("unsupported archive metadata".into());
    }
    Ok(metadata)
}

fn validate_archive_root(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_symlink() || !metadata.is_dir() => {
            Err("archive root must be a real directory, not a symlink".into())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("inspect archive root: {error}")),
    }
}

fn preflight(source: &Path, destination: &Path, bare: bool) -> Result<()> {
    if !bare {
        return lifecycle::preflight_move(source, destination);
    }
    lifecycle::vacant(destination)?;
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("inspect bare repository: {error}"))?;
    if metadata.is_symlink()
        || !metadata.is_dir()
        || process::git(source, &["rev-parse", "--is-bare-repository"])? != "true"
    {
        return Err("expected a standalone bare repository".into());
    }
    if source.join("objects/info/alternates").exists() {
        return Err("bare repository borrows an object database; cannot move it safely".into());
    }
    lifecycle::check_metadata_links(source)?;
    if source.join("commondir").exists()
        || process::git_optional(source, &["config", "--local", "--get", "core.worktree"])?
            .is_some()
    {
        return Err("bare repository uses shared or separate worktree metadata".into());
    }
    for marker in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "BISECT_LOG",
        "rebase-merge",
        "rebase-apply",
        "sequencer",
    ] {
        if source.join(marker).exists() {
            return Err(
                "finish the current Git operation before archiving the bare repository".into(),
            );
        }
    }
    let worktrees = process::git(source, &["worktree", "list", "--porcelain", "-z"])?;
    if worktrees
        .split('\0')
        .filter(|field| field.starts_with("worktree "))
        .count()
        != 1
    {
        return Err("bare repository has linked worktrees; cannot move it safely".into());
    }
    let parent = lifecycle::existing_parent(destination)?;
    let canonical_source = source.canonicalize().map_err(|error| error.to_string())?;
    let canonical_parent = parent.canonicalize().map_err(|error| error.to_string())?;
    if canonical_parent.starts_with(&canonical_source) {
        return Err("destination cannot be inside the source repository".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.dev()
            != fs::metadata(&canonical_parent)
                .map_err(|error| error.to_string())?
                .dev()
        {
            return Err("moving across filesystems is not atomic".into());
        }
    }
    Ok(())
}

fn move_repository(source: &Path, destination: &Path, bare: bool) -> Result<()> {
    preflight(source, destination, bare)?;
    let parent = destination.parent().ok_or("destination has no parent")?;
    fs::create_dir_all(parent).map_err(|error| format!("create destination parent: {error}"))?;
    lifecycle::rename_new(source, destination)
}

fn render(reports: &[Report], json: bool) -> Result<u8> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(reports).map_err(|error| error.to_string())?
        );
    } else {
        for report in reports {
            println!(
                "{}\t{}\t{}",
                report.action,
                report.id,
                report.path.display()
            );
        }
    }
    Ok(0)
}
