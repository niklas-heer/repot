//! Checkout creation and atomic, non-overwriting relocation.

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::{Result, navigation, process};

/// Where a new project's first files come from: a repository you could clone
/// (owner/name, host/owner/name, URL) or a local checkout.
pub struct Template {
    source: String,
    branch: Option<String>,
}

impl Template {
    pub fn resolve(config: &Config, input: &str) -> Result<Self> {
        let local = input
            .strip_prefix("~/")
            .and_then(|rest| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(rest)))
            .unwrap_or_else(|| PathBuf::from(input));
        if local.join(".git").exists() {
            let path = local
                .canonicalize()
                .map_err(|error| format!("resolve template: {error}"))?;
            return Ok(Self {
                source: path.to_string_lossy().into_owned(),
                branch: None,
            });
        }
        let spec = crate::remote_spec::resolve(config, input, false, false)
            .map_err(|error| format!("template: {error}"))?;
        Ok(Self {
            source: spec.url,
            branch: spec.branch,
        })
    }

    /// Copy the template's committed files into `checkout`, without its history
    /// and without committing anything.
    pub fn fill(&self, checkout: &Path) -> Result<()> {
        let parent = checkout.parent().ok_or("destination has no parent")?;
        let scratch = tempfile::Builder::new()
            .prefix(".repot-template-")
            .tempdir_in(parent)
            .map_err(|error| format!("prepare template: {error}"))?;
        let copy = scratch.path().join("template");
        let mut args: Vec<std::ffi::OsString> = [
            "clone",
            "--quiet",
            "--no-recurse-submodules",
            "--depth",
            "1",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        if let Some(branch) = &self.branch {
            args.extend(["--branch".into(), branch.into()]);
        }
        args.extend([
            "--".into(),
            self.source.clone().into(),
            copy.clone().into_os_string(),
        ]);
        let args: Vec<&std::ffi::OsStr> = args.iter().map(std::ffi::OsString::as_os_str).collect();
        let output = process::run("git", &args, parent, std::time::Duration::from_mins(2))?;
        if !output.success {
            return Err("cannot clone the template; check the name and your access".into());
        }
        for entry in fs::read_dir(&copy).map_err(|error| format!("read template: {error}"))? {
            let entry = entry.map_err(|error| format!("read template: {error}"))?;
            if entry.file_name() == ".git" {
                continue;
            }
            fs::rename(entry.path(), checkout.join(entry.file_name()))
                .map_err(|error| format!("copy template: {error}"))?;
        }
        Ok(())
    }
}

pub fn new_project(
    config: &Config,
    name: &str,
    namespace: &str,
    dry_run: bool,
    template: Option<&str>,
) -> Result<()> {
    let template = template
        .map(|input| Template::resolve(config, input))
        .transpose()?;
    validate_component(name)?;
    validate_component(namespace)?;
    let root = config.roots.last().ok_or("no repository root configured")?;
    let target = root.join("local").join(namespace).join(name);
    crate::discovery::validate_public_path(&target)?;
    vacant(&target)?;
    let parent = target.parent().ok_or("destination has no parent")?;
    existing_parent(&target)?;
    if dry_run {
        println!(
            "would create scratch repository at {}{}",
            target.display(),
            template
                .as_ref()
                .map(|template| format!(" from template {}", template.source))
                .unwrap_or_default()
        );
        return Ok(());
    }
    fs::create_dir_all(parent).map_err(|error| format!("create scratch parent: {error}"))?;
    let staging = tempfile::Builder::new()
        .prefix(".repot-new-")
        .tempdir_in(parent)
        .map_err(|error| format!("prepare scratch checkout: {error}"))?;
    process::git(
        staging.path(),
        &["init", "--template=", "--initial-branch=main"],
    )?;
    if let Some(template) = &template {
        template.fill(staging.path())?;
    }
    rename_new(staging.path(), &target)?;
    crate::ui::done(&format!("created scratch project {}", target.display()));
    navigation::handoff(&target.canonicalize().map_err(|error| error.to_string())?)
}

fn validate_component(value: &str) -> Result<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.starts_with('-')
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    {
        return Err("names must contain only letters, digits, dots, underscores or hyphens, and cannot start with a hyphen or be . or ..".into());
    }
    Ok(())
}

pub fn checkout_root(path: &Path) -> Result<PathBuf> {
    let root = process::git(path, &["rev-parse", "--show-toplevel"])?;
    PathBuf::from(root)
        .canonicalize()
        .map_err(|error| format!("resolve checkout: {error}"))
}

pub fn vacant(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(format!("destination already exists: {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("inspect destination: {error}")),
    }
}

pub fn preflight_move(source: &Path, destination: &Path) -> Result<()> {
    vacant(destination)?;
    let source = source.canonicalize().map_err(|error| error.to_string())?;
    let git_metadata =
        fs::symlink_metadata(source.join(".git")).map_err(|error| error.to_string())?;
    if source != checkout_root(&source)? || !git_metadata.is_dir() {
        return Err(
            "only standalone checkout roots can move; use adopt --register for linked worktrees"
                .into(),
        );
    }
    if source.join(".git/objects/info/alternates").exists() {
        return Err(
            "checkout borrows an external object database; register it in place instead".into(),
        );
    }
    if source.join(".git/commondir").exists() {
        return Err("checkout has a shared Git directory; register it in place instead".into());
    }
    check_metadata_links(&source.join(".git"))?;
    let worktrees = process::git(&source, &["worktree", "list", "--porcelain", "-z"])?;
    if worktrees
        .split('\0')
        .filter(|field| field.starts_with("worktree "))
        .count()
        != 1
    {
        return Err("checkout has linked worktrees; register it in place instead".into());
    }
    let entries = process::git(&source, &["ls-files", "--stage", "-z"])?;
    if entries
        .split('\0')
        .any(|entry| entry.starts_with("160000 "))
        || source.join(".git/modules").exists()
    {
        return Err("checkout contains submodules; register it in place instead".into());
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
        if source.join(".git").join(marker).exists() {
            return Err("finish the current Git operation before moving the checkout".into());
        }
    }
    if process::git_optional(&source, &["config", "--local", "--get", "core.worktree"])?.is_some() {
        return Err("checkout uses a separate worktree path; register it in place instead".into());
    }
    let parent = existing_parent(destination)?;
    let canonical_parent = parent.canonicalize().map_err(|error| error.to_string())?;
    if canonical_parent.starts_with(&source) {
        return Err("destination cannot be inside the source checkout".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let old = fs::metadata(&source).map_err(|error| error.to_string())?;
        let new = fs::metadata(&canonical_parent).map_err(|error| error.to_string())?;
        if old.dev() != new.dev() {
            return Err(
                "moving across filesystems is not atomic; use adopt --register instead".into(),
            );
        }
    }
    Ok(())
}

pub fn check_metadata_links(git_dir: &Path) -> Result<()> {
    let mut pending = vec![git_dir.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in
            fs::read_dir(directory).map_err(|error| format!("inspect Git metadata: {error}"))?
        {
            let entry = entry.map_err(|error| error.to_string())?;
            let kind = entry.file_type().map_err(|error| error.to_string())?;
            if kind.is_symlink() {
                return Err(
                    "checkout has symlinked Git metadata; register it in place instead".into(),
                );
            }
            if kind.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    Ok(())
}

pub fn existing_parent(path: &Path) -> Result<&Path> {
    let mut parent = path.parent().ok_or("destination has no parent")?;
    loop {
        match fs::symlink_metadata(parent) {
            Ok(_) if parent.is_dir() => return Ok(parent),
            Ok(_) => return Err("destination parent is not a directory".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                parent = parent
                    .parent()
                    .ok_or("destination has no existing parent")?;
            }
            Err(error) => return Err(format!("inspect destination parent: {error}")),
        }
    }
}

pub fn move_checkout(source: &Path, destination: &Path) -> Result<()> {
    preflight_move(source, destination)?;
    let parent = destination.parent().ok_or("destination has no parent")?;
    fs::create_dir_all(parent).map_err(|error| format!("create destination parent: {error}"))?;
    rename_new(source, destination)?;
    crate::ui::done(&format!("moved to {}", destination.display()));
    Ok(())
}

/// The kernel refuses an existing destination, including a racing empty directory.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn rename_new(source: &Path, destination: &Path) -> Result<()> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        source,
        rustix::fs::CWD,
        destination,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(|error| format!("atomic move refused (source preserved): {error}"))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn rename_new(_source: &Path, _destination: &Path) -> Result<()> {
    Err("atomic no-overwrite moves currently require Linux or macOS".into())
}
