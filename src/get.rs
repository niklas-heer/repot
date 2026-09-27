//! Safe clone/import workflows with ghq-compatible options.

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{IsTerminal, Read};
use std::path::Path;
use std::time::Duration;

use clap::{Args, ValueEnum};
use serde::Serialize;

use crate::Result;
use crate::config::Config;
use crate::remote_spec::{self, Spec};
use crate::{lifecycle, navigation, process, status, sync, ui};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Partial {
    Blobless,
    Treeless,
}

#[derive(Debug, Args)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "these independent flags preserve the ghq command interface"
)]
pub struct Options {
    /// URL, host/owner/repo, owner/repo or repo; read newline-separated stdin if omitted.
    pub repositories: Vec<String>,
    /// Fast-forward checkouts that already exist, when that is safe.
    #[arg(short, long)]
    pub update: bool,
    /// Use SSH instead of HTTPS.
    #[arg(short = 'p', long = "ssh", visible_alias = "p")]
    pub ssh: bool,
    /// Clone only the latest commit.
    #[arg(long)]
    pub shallow: bool,
    /// Enter the first checkout through the shell wrapper.
    #[arg(short = 'l', long)]
    pub look: bool,
    /// Version control system; only git is supported.
    #[arg(long, hide = true)]
    pub vcs: Option<String>,
    /// Print nothing on success.
    #[arg(short = 's', long)]
    pub silent: bool,
    /// Do not clone submodules.
    #[arg(long)]
    pub no_recursive: bool,
    /// Check out this branch instead of the remote default.
    #[arg(short = 'b', long)]
    pub branch: Option<String>,
    /// Clone several repositories at once.
    #[arg(short = 'P', long)]
    pub parallel: bool,
    /// Maximum concurrent clones with --parallel.
    #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u8).range(1..=32))]
    pub jobs: u8,
    /// Create a bare repository without a working tree.
    #[arg(long)]
    pub bare: bool,
    /// Fetch file contents or trees on demand instead of up front.
    #[arg(long, value_enum)]
    pub partial: Option<Partial>,
    /// Show what would be cloned without changing anything.
    #[arg(long)]
    pub dry_run: bool,
    /// Print machine-readable results.
    #[arg(long)]
    pub json: bool,
    /// Maximum duration of each Git subprocess, in seconds.
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u64).range(1..=3600))]
    pub timeout: u64,
}

#[derive(Debug, Args)]
pub struct CreateOptions {
    /// Repository location: owner/name, host/owner/name or a URL.
    pub repository: String,
    /// Version control system; only git is supported.
    #[arg(long, hide = true)]
    pub vcs: Option<String>,
    /// Create a bare repository without a working tree.
    #[arg(long)]
    pub bare: bool,
    /// Show where the repository would go without creating it.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Serialize)]
struct Report {
    path: std::path::PathBuf,
    action: String,
    reason: String,
    #[serde(skip)]
    code: u8,
}

pub fn run(config: &Config, options: &Options) -> Result<u8> {
    remote_spec::git_vcs(options.vcs.as_deref())?;
    let targets = inputs(options)?;
    let mut seen = BTreeSet::new();
    let mut specs = Vec::new();
    for input in targets {
        let mut spec = remote_spec::resolve(config, &input, options.ssh, false)?;
        if options.bare {
            spec.path = config.destination_bare(&spec.url)?;
        }
        if let Some(branch) = &options.branch {
            spec.branch = Some(branch.clone());
        }
        validate_branch(spec.branch.as_deref())?;
        if seen.insert(spec.path.clone()) {
            specs.push(spec);
        }
    }
    let jobs = if options.parallel {
        usize::from(options.jobs)
    } else {
        1
    };
    let spinner = (!options.json && !options.silent && !specs.is_empty()).then(|| {
        ui::Progress::start(
            if options.dry_run {
                "Checking"
            } else {
                "Cloning"
            },
            specs.len(),
        )
    });
    let progress = spinner.as_ref();
    let reports = crate::work::parallel(&specs, jobs, progress, |spec| {
        if let Some(progress) = progress {
            progress.working_on(&ui::repository_name(&spec.path, &config.roots));
        }
        get(config, spec, options, true).unwrap_or_else(|reason| Report {
            path: spec.path.clone(),
            action: "failed".into(),
            reason,
            code: 1,
        })
    })?;
    drop(spinner);
    let code = if reports.iter().any(|report| report.code == 1) {
        1
    } else if reports.iter().any(|report| report.code == 3) {
        3
    } else {
        0
    };
    print_reports(config, options, &reports)?;
    if code == 0
        && options.look
        && !options.dry_run
        && let Some(first) = reports.first()
        && std::env::var_os("REPOT_CD_FILE").is_some()
    {
        navigation::handoff(&first.path)?;
    }
    Ok(code)
}

/// Paths go to stdout for scripts; a readable line per repository goes to stderr.
fn print_reports(config: &Config, options: &Options, reports: &[Report]) -> Result<()> {
    if options.json {
        println!(
            "{}",
            serde_json::to_string_pretty(reports).map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    for report in reports {
        if report.code == 0 {
            println!("{}", report.path.display());
        }
        if !options.silent || report.code != 0 {
            let (symbol, style) = match report.code {
                0 => ("✓", ui::GOOD),
                3 => ("●", ui::WARN),
                _ => ("✗", ui::BAD),
            };
            ui::note(
                symbol,
                style,
                &format!(
                    "{} {} {}",
                    report.action,
                    ui::repository_name(&report.path, &config.roots),
                    ui::Paint::stderr().paint(ui::DIM, format!("· {}", report.reason))
                ),
            );
        }
    }
    Ok(())
}

fn inputs(options: &Options) -> Result<Vec<String>> {
    if !options.repositories.is_empty() {
        return Ok(options.repositories.clone());
    }
    if std::io::stdin().is_terminal() {
        return Err("supply a repository or pipe a repository list to get".into());
    }
    let mut input = String::new();
    std::io::stdin()
        .take(16 * 1024 * 1024 + 1)
        .read_to_string(&mut input)
        .map_err(|_| "cannot read repository list")?;
    if input.len() > 16 * 1024 * 1024 {
        return Err("repository list is too large".into());
    }
    let values: Vec<_> = input
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    if values.is_empty() {
        return Err("repository list is empty".into());
    }
    Ok(values)
}

fn validate_branch(branch: Option<&str>) -> Result<()> {
    if let Some(branch) = branch {
        if branch.is_empty() || branch.starts_with('-') || branch.chars().any(char::is_control) {
            return Err("invalid branch name".into());
        }
        let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
        process::git(&cwd, &["check-ref-format", "--branch", branch])
            .map_err(|_| "invalid branch name".to_owned())?;
    }
    Ok(())
}

fn report(spec: &Spec, action: &str, reason: &str, code: u8) -> Report {
    Report {
        path: spec.path.clone(),
        action: action.into(),
        reason: reason.into(),
        code,
    }
}

fn get(config: &Config, spec: &Spec, options: &Options, allow_vanity: bool) -> Result<Report> {
    let configured_vcs = if options.vcs.is_none() {
        crate::config::git_url_value("ghq.vcs", &spec.url)?
    } else {
        None
    };
    remote_spec::git_vcs(configured_vcs.as_deref())?;
    let allow_vanity = allow_vanity && options.vcs.is_none() && configured_vcs.is_none();
    match fs::symlink_metadata(&spec.path) {
        Ok(metadata) if metadata.is_dir() && !metadata.is_symlink() => {
            let bare = process::git(&spec.path, &["rev-parse", "--is-bare-repository"])? == "true";
            if bare != options.bare
                || (!bare
                    && lifecycle::checkout_root(&spec.path)?
                        != spec
                            .path
                            .canonicalize()
                            .map_err(|error| error.to_string())?)
            {
                return Err("destination is not the requested kind of repository".into());
            }
            if options.update {
                return update(config, spec, options);
            }
            return Ok(report(
                spec,
                "exists",
                "existing repository left untouched",
                0,
            ));
        }
        Ok(_) => return Err("destination already exists and is not a standalone directory".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("inspect clone destination: {error}")),
    }
    lifecycle::existing_parent(&spec.path)?;
    if options.dry_run {
        return Ok(report(
            spec,
            "clone",
            "would clone into a private staging directory; a failed HTTP clone may resolve vanity metadata",
            0,
        ));
    }
    let parent = spec
        .path
        .parent()
        .ok_or("clone destination has no parent")?;
    let anchor = lifecycle::existing_parent(&spec.path)?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let stage = tempfile::Builder::new()
        .prefix(".repot-clone-")
        .tempdir_in(parent)
        .map_err(|error| error.to_string())?;
    let checkout = stage.path().join("checkout");
    let args = clone_args(spec, options, &checkout);
    if let Err(error) = git_args(parent, &args, options.timeout) {
        // The failed checkout belongs exclusively to this temporary staging
        // directory. Drop it before resolving or attempting another destination.
        drop(stage);
        remove_empty_created_parents(parent, anchor);
        if allow_vanity
            && let Some(vanity) =
                crate::remote_extra::vanity(&spec.url, Duration::from_secs(options.timeout))?
        {
            let import_url = format!("https://{}", vanity.prefix);
            let path = if options.bare {
                config.destination_bare(&import_url)?
            } else {
                config.destination(&import_url)?
            };
            let resolved = Spec {
                url: vanity.url,
                path,
                branch: spec.branch.clone(),
            };
            return get(config, &resolved, options, false);
        }
        return Err(error);
    }
    lifecycle::rename_new(&checkout, &spec.path)?;
    Ok(report(
        spec,
        "cloned",
        "clone completed and installed atomically",
        0,
    ))
}

fn remove_empty_created_parents(mut path: &Path, anchor: &Path) {
    while path != anchor {
        if fs::remove_dir(path).is_err() {
            break;
        }
        let Some(parent) = path.parent() else {
            break;
        };
        path = parent;
    }
}

fn clone_args(spec: &Spec, options: &Options, checkout: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["clone", "--template=", "--no-local"]
        .into_iter()
        .map(Into::into)
        .collect();
    if options.shallow {
        args.extend(["--depth".into(), "1".into()]);
    }
    if options.bare {
        args.push("--bare".into());
    }
    if !options.no_recursive && !options.bare {
        args.push("--recurse-submodules".into());
    }
    if let Some(branch) = &spec.branch {
        args.extend(["--branch".into(), branch.into(), "--single-branch".into()]);
    }
    if let Some(partial) = options.partial {
        args.push(
            match partial {
                Partial::Blobless => "--filter=blob:none",
                Partial::Treeless => "--filter=tree:0",
            }
            .into(),
        );
    }
    args.extend([
        "--".into(),
        spec.url.clone().into(),
        checkout.as_os_str().to_owned(),
    ]);
    args
}

fn update(config: &Config, spec: &Spec, options: &Options) -> Result<Report> {
    if options.bare {
        if options.dry_run {
            return Ok(report(
                spec,
                "fetch",
                "would atomically fetch fast-forward refs without pruning",
                0,
            ));
        }
        let args: Vec<OsString> = [
            "fetch",
            "--atomic",
            "--refmap=",
            "--no-tags",
            "--no-prune",
            "--no-prune-tags",
            "--no-recurse-submodules",
            "--no-write-fetch-head",
            "--",
            &spec.url,
            "refs/heads/*:refs/heads/*",
            "refs/tags/*:refs/tags/*",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        git_args(&spec.path, &args, options.timeout)?;
        return Ok(report(
            spec,
            "updated",
            "bare refs fetched without force or pruning",
            0,
        ));
    }
    let settings = status::Options {
        json: false,
        jobs: 1,
        timeout: Duration::from_secs(options.timeout),
        no_fetch: options.dry_run,
    };
    let mut result = status::inspect(&spec.path, config, &settings, !options.dry_run)?;
    if result.action == "pull" && !options.dry_run {
        sync::apply(&settings, &mut result)?;
    }
    let code = if result.failed {
        1
    } else if matches!(result.action.as_str(), "none" | "pull") {
        0
    } else {
        3
    };
    Ok(report(
        spec,
        if options.dry_run {
            "plan"
        } else if result.applied {
            "updated"
        } else {
            &result.action
        },
        &result.reason,
        code,
    ))
}

fn git_args(path: &Path, args: &[OsString], timeout: u64) -> Result<()> {
    let args: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
    let output = process::run("git", &args, path, Duration::from_secs(timeout))?;
    if !output.success {
        return Err("Git operation failed; existing repositories preserved".into());
    }
    Ok(())
}

pub fn create(config: &Config, options: &CreateOptions) -> Result<u8> {
    remote_spec::git_vcs(options.vcs.as_deref())?;
    let mut spec = remote_spec::resolve(config, &options.repository, false, true)?;
    if spec.branch.is_some() {
        return Err("create does not accept a remote branch suffix".into());
    }
    if options.bare {
        spec.path = config.destination_bare(&spec.url)?;
    }
    lifecycle::vacant(&spec.path)?;
    lifecycle::existing_parent(&spec.path)?;
    if options.dry_run {
        println!("would create {}", spec.path.display());
        return Ok(0);
    }
    let parent = spec.path.parent().ok_or("destination has no parent")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let stage = tempfile::Builder::new()
        .prefix(".repot-create-")
        .tempdir_in(parent)
        .map_err(|error| error.to_string())?;
    let mut args = vec!["init", "--template="];
    if options.bare {
        args.push("--bare");
    }
    process::git(stage.path(), &args)?;
    lifecycle::rename_new(stage.path(), &spec.path)?;
    if std::env::var_os("REPOT_CD_FILE").is_some() {
        navigation::handoff(&spec.path)?;
    } else {
        println!("{}", spec.path.display());
    }
    Ok(0)
}
