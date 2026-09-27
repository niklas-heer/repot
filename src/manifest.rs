//! Stray discovery, adoption and atomic manifest-backed restoration.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::config::{Config, Manifest, RepoEntry, remote_parts};
use crate::{Result, discovery, lifecycle, navigation, process};

#[derive(Serialize)]
struct Report {
    path: PathBuf,
    action: &'static str,
    reason: String,
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
                report.path.display(),
                report.reason
            );
        }
    }
    Ok(u8::from(
        reports.iter().any(|report| report.action == "error"),
    ))
}

pub fn find(config: &Config, path: &Path, json: bool) -> Result<u8> {
    let known: BTreeSet<_> = discovery::discover(config)?
        .into_iter()
        .map(|repo| repo.path)
        .collect();
    let roots: Vec<_> = config
        .roots
        .iter()
        .filter_map(|path| path.canonicalize().ok())
        .collect();
    let reports: Vec<_> = discovery::find(path)?
        .into_iter()
        .filter(|repo| {
            !known.contains(&repo.path) && !roots.iter().any(|root| repo.path.starts_with(root))
        })
        .map(|repo| Report {
            path: repo.path,
            action: "found",
            reason: "unregistered checkout".into(),
        })
        .collect();
    render(&reports, json)
}

pub fn adopt(
    config: &Config,
    path: &Path,
    register: bool,
    dry_run: bool,
    json: bool,
) -> Result<u8> {
    let source = path
        .canonicalize()
        .map_err(|error| format!("resolve checkout: {error}"))?;
    discovery::validate_public_path(&source)?;
    if lifecycle::checkout_root(&source)? != source {
        return Err("adopt expects the root of a Git checkout".into());
    }
    let url = remote(&source)?;
    let destination = if register {
        source.clone()
    } else {
        if url.is_empty() {
            return Err("checkout has no remote; use adopt --register to keep it in place".into());
        }
        config.destination(&url)?
    };
    let moving = source
        != destination
            .canonicalize()
            .unwrap_or_else(|_| destination.clone());
    if moving {
        lifecycle::preflight_move(&source, &destination)?;
    }
    let has_remote = !url.is_empty();
    let registered_path = if register {
        Some(portable_path(&source)?)
    } else {
        None
    };
    let entry = RepoEntry {
        url,
        path: registered_path,
        restore: has_remote,
    };
    validate_entry(&entry)?;
    let manifest_path = manifest_target(&config.manifest_path)?;
    let (mut latest, mut original) = read_manifest(config, &manifest_path)?;
    let _lock = if dry_run {
        None
    } else {
        Some(lock_manifest(&manifest_path)?)
    };
    if !dry_run {
        (latest, original) = read_manifest(config, &manifest_path)?;
    }
    let document = edit_manifest(&latest, &original, &source, &destination, &entry)?;
    let action = if moving { "move" } else { "register" };
    if dry_run {
        return render(
            &[Report {
                path: destination,
                action,
                reason: format!("would {action} checkout and update manifest"),
            }],
            json,
        );
    }
    let staged = stage_manifest(&manifest_path, &document)?;
    unchanged_manifest(&manifest_path, &original)?;
    if moving {
        lifecycle::move_checkout(&source, &destination)?;
    }
    unchanged_manifest(&manifest_path, &original).map_err(|error| {
        if moving {
            format!(
                "checkout moved to {} but {error}; register that location to recover",
                destination.display()
            )
        } else {
            error
        }
    })?;
    staged.persist(&manifest_path).map_err(|error| {
        if moving {
            format!("checkout moved to {} but manifest could not be saved: {}; register that location to recover", destination.display(), error.error)
        } else {
            format!("could not save manifest: {}", error.error)
        }
    })?;
    if std::env::var_os("REPOT_CD_FILE").is_some() {
        navigation::handoff(&destination)?;
    }
    render(
        &[Report {
            path: destination,
            action,
            reason: "checkout registered in manifest".into(),
        }],
        json,
    )
}

fn portable_path(path: &Path) -> Result<String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .and_then(|home| home.canonicalize().ok());
    let path = home
        .and_then(|home| {
            path.strip_prefix(home)
                .ok()
                .map(|relative| Path::new("~").join(relative))
        })
        .unwrap_or_else(|| path.to_path_buf());
    Ok(path
        .to_str()
        .ok_or("manifest paths must be valid UTF-8")?
        .replace('$', "$$"))
}

fn unchanged_manifest(path: &Path, original: &str) -> Result<()> {
    let current = match fs::read_to_string(path) {
        Ok(current) => current,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("manifest cannot be reread: {error}")),
    };
    if current != original {
        return Err("manifest changed in another editor; no manifest changes were saved".into());
    }
    Ok(())
}

fn remote(path: &Path) -> Result<String> {
    let remotes = process::git(path, &["remote"])?;
    if remotes.is_empty() {
        return Ok(String::new());
    }
    let selected = if remotes.lines().any(|name| name == "origin") {
        "origin"
    } else {
        let mut names = remotes.lines();
        let selected = names.next().ok_or("no remote found")?;
        if names.next().is_some() {
            return Err("multiple remotes without origin; choose an origin before adopting".into());
        }
        selected
    };
    process::git(path, &["remote", "get-url", selected])
}

fn manifest_target(path: &Path) -> Result<PathBuf> {
    match fs::symlink_metadata(path) {
        Ok(_) => path
            .canonicalize()
            .map_err(|error| format!("resolve manifest: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(path.to_path_buf()),
        Err(error) => Err(format!("inspect manifest: {error}")),
    }
}

fn lock_manifest(path: &Path) -> Result<File> {
    let parent = path.parent().ok_or("manifest has no parent")?;
    fs::create_dir_all(parent).map_err(|error| format!("create manifest parent: {error}"))?;
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".lock");
    let lock = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|error| format!("open manifest lock: {error}"))?;
    let started = Instant::now();
    loop {
        match lock.try_lock() {
            Ok(()) => return Ok(lock),
            Err(fs::TryLockError::WouldBlock) if started.elapsed() < Duration::from_secs(5) => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return Err("manifest is locked by another command; retry shortly".into()),
        }
    }
}

fn read_manifest(config: &Config, target: &Path) -> Result<(Config, String)> {
    let text = match fs::read_to_string(target) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("read manifest: {error}")),
    };
    let manifest: Manifest = crate::manifest_format::parse(&config.manifest_path, &text)
        .map_err(|_| "manifest is invalid; no changes were made".to_owned())?;
    for entry in &manifest.repo {
        validate_entry(entry)?;
    }
    Ok((
        Config {
            roots: config.roots.clone(),
            manifest_path: config.manifest_path.clone(),
            manifest,
        },
        text,
    ))
}

fn entry_path(config: &Config, entry: &RepoEntry) -> Result<PathBuf> {
    entry.path.as_ref().map_or_else(
        || config.destination(&entry.url),
        |path| config.expand_path(path),
    )
}

fn edit_manifest(
    config: &Config,
    original: &str,
    source: &Path,
    destination: &Path,
    entry: &RepoEntry,
) -> Result<String> {
    let mut matching = None;
    for (index, known) in config.manifest.repo.iter().enumerate() {
        let path = entry_path(config, known)?;
        let path = path.canonicalize().unwrap_or(path);
        if (path == source || path == destination) && matching.replace(index).is_some() {
            return Err(
                "manifest contains duplicate registrations; resolve them before adopting".into(),
            );
        }
    }
    crate::manifest_format::edit(&config.manifest_path, original, matching, entry)
}

fn stage_manifest(path: &Path, text: &str) -> Result<tempfile::NamedTempFile> {
    let parent = path.parent().ok_or("manifest has no parent")?;
    let mut staged = tempfile::Builder::new()
        .prefix(".repot-manifest-")
        .tempfile_in(parent)
        .map_err(|error| format!("stage manifest: {error}"))?;
    if let Ok(metadata) = fs::metadata(path) {
        staged
            .as_file()
            .set_permissions(metadata.permissions())
            .map_err(|error| format!("preserve manifest permissions: {error}"))?;
    }
    staged
        .write_all(text.as_bytes())
        .map_err(|error| format!("write manifest: {error}"))?;
    staged
        .as_file()
        .sync_all()
        .map_err(|error| format!("sync manifest: {error}"))?;
    Ok(staged)
}

fn validate_entry(entry: &RepoEntry) -> Result<()> {
    if entry.url.is_empty() && entry.path.is_some() && !entry.restore {
        return Ok(());
    }
    clone_url(entry).map(|_| ())
}

fn clone_url(entry: &RepoEntry) -> Result<String> {
    let url = &entry.url;
    if entry.path.is_some() {
        let local = url.strip_prefix("file://").unwrap_or(url);
        if Path::new(local).is_absolute() && !local.contains(['\0', '\n', '\r']) {
            return Ok(url.clone());
        }
    }
    remote_parts(url)?;
    if !url.contains(':') {
        return Ok(format!("https://{url}"));
    }
    Ok(url.clone())
}

pub fn restore(
    config: &Config,
    dry_run: bool,
    json: bool,
    timeout: Duration,
    jobs: usize,
) -> Result<u8> {
    let restore_entry = |entry: &RepoEntry| match entry_path(config, entry) {
        Ok(path) => match restore_one(entry, &path, dry_run, timeout) {
            Ok((action, reason)) => Report {
                path,
                action,
                reason: reason.into(),
            },
            Err(reason) => Report {
                path,
                action: "error",
                reason,
            },
        },
        Err(reason) => Report {
            path: config.manifest_path.clone(),
            action: "error",
            reason,
        },
    };
    // Entries stage their own clones before an atomic, non-overwriting rename,
    // so unrelated destinations clone side by side. Entries whose destinations
    // are equal or nested run one after another, in manifest order, exactly as
    // a sequential restore would.
    let groups = destination_groups(config);
    let progress = (!json && !dry_run && !config.manifest.repo.is_empty())
        .then(|| crate::ui::Progress::start("Restoring", groups.len()));
    let grouped = crate::work::parallel(&groups, jobs, progress.as_ref(), |group| {
        group
            .iter()
            .filter_map(|index| config.manifest.repo.get(*index))
            .map(restore_entry)
            .zip(group.iter().copied())
            .collect::<Vec<_>>()
    })?;
    let mut reports: Vec<(usize, Report)> = grouped
        .into_iter()
        .flatten()
        .map(|(report, index)| (index, report))
        .collect();
    reports.sort_by_key(|(index, _)| *index);
    let reports: Vec<Report> = reports.into_iter().map(|(_, report)| report).collect();
    drop(progress);
    render(&reports, json)
}

/// Manifest indices grouped so that equal or nested destinations share a group.
fn destination_groups(config: &Config) -> Vec<Vec<usize>> {
    let paths: Vec<Option<PathBuf>> = config
        .manifest
        .repo
        .iter()
        .map(|entry| entry_path(config, entry).ok())
        .collect();
    let related = |left: usize, right: usize| match (paths.get(left), paths.get(right)) {
        (Some(Some(left)), Some(Some(right))) => left.starts_with(right) || right.starts_with(left),
        _ => false,
    };
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for index in 0..paths.len() {
        let joined: Vec<usize> = groups
            .iter()
            .enumerate()
            .filter(|(_, group)| group.iter().any(|member| related(*member, index)))
            .map(|(position, _)| position)
            .collect();
        let mut merged = vec![index];
        for position in joined.into_iter().rev() {
            merged.extend(groups.remove(position));
        }
        merged.sort_unstable();
        groups.push(merged);
    }
    groups.sort_by_key(|group| group.first().copied());
    groups
}

fn restore_one(
    entry: &RepoEntry,
    target: &Path,
    dry_run: bool,
    timeout: Duration,
) -> Result<(&'static str, &'static str)> {
    if !entry.restore {
        return Ok(("skip", "restoration disabled"));
    }
    discovery::validate_public_path(target)?;
    match fs::symlink_metadata(target) {
        Ok(_) => return Ok(("skip", "destination already exists; left untouched")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("inspect destination: {error}")),
    }
    let url = clone_url(entry)?;
    let parent = target.parent().ok_or("destination has no parent")?;
    lifecycle::existing_parent(target)?;
    if dry_run {
        return Ok(("clone", "would clone missing checkout"));
    }
    fs::create_dir_all(parent).map_err(|error| format!("create restore parent: {error}"))?;
    let staging = tempfile::Builder::new()
        .prefix(".repot-restore-")
        .tempdir_in(parent)
        .map_err(|error| format!("prepare clone: {error}"))?;
    let arguments = [
        OsStr::new("-c"),
        OsStr::new("protocol.allow=never"),
        OsStr::new("-c"),
        OsStr::new("protocol.file.allow=always"),
        OsStr::new("-c"),
        OsStr::new("protocol.https.allow=always"),
        OsStr::new("-c"),
        OsStr::new("protocol.http.allow=always"),
        OsStr::new("-c"),
        OsStr::new("protocol.ssh.allow=always"),
        OsStr::new("clone"),
        OsStr::new("--template="),
        OsStr::new("--no-recurse-submodules"),
        OsStr::new("--no-local"),
        OsStr::new("--"),
        OsStr::new(&url),
        staging.path().as_os_str(),
    ];
    if !process::run("git", &arguments, parent, timeout)?.success {
        return Err("clone failed; destination was not created".into());
    }
    lifecycle::rename_new(staging.path(), target)?;
    Ok(("clone", "missing checkout restored"))
}
