//! Explicit publication with remote verification and recoverable local moves.

use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Args, ValueEnum};
use serde_json::Value;

use crate::Result;
use crate::config::{Config, remote_parts};
use crate::{lifecycle, navigation, process};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Forge {
    Github,
    Gitlab,
}

impl Forge {
    const fn command(self) -> &'static str {
        match self {
            Self::Github => "gh",
            Self::Gitlab => "glab",
        }
    }

    const fn host(self) -> &'static str {
        match self {
            Self::Github => "github.com",
            Self::Gitlab => "gitlab.com",
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Visibility {
    Public,
    Private,
}

impl Visibility {
    const fn name(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
        }
    }

    const fn flag(self) -> &'static str {
        match self {
            Self::Public => "--public",
            Self::Private => "--private",
        }
    }
}

#[derive(Debug, Args)]
pub struct Options {
    /// Explicit owner/name, or group/subgroup/name on GitLab.
    pub repo: String,
    #[arg(long, value_enum, default_value = "github")]
    pub forge: Forge,
    /// Forge hostname (defaults to github.com or gitlab.com).
    #[arg(long)]
    pub host: Option<String>,
    /// Explicit visibility for the newly published repository.
    #[arg(long, value_enum)]
    pub visibility: Visibility,
    /// Check prerequisites and describe publication without changing anything.
    #[arg(long)]
    pub dry_run: bool,
    /// Continue publication using an existing, matching origin remote.
    #[arg(long)]
    pub resume: bool,
    /// Maximum duration of each forge or Git subprocess, in seconds.
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u64).range(1..=3600))]
    pub timeout: u64,
}

struct Target {
    host: String,
    url: String,
    destination: PathBuf,
}

#[derive(PartialEq, Eq)]
struct Snapshot {
    branch: String,
    head: String,
}

pub fn run(config: &Config, options: &Options) -> Result<u8> {
    let cwd = env::current_dir().map_err(|error| format!("read current directory: {error}"))?;
    let source = lifecycle::checkout_root(&cwd)?;
    let target = target(config, options)?;
    lifecycle::preflight_move(&source, &target.destination)?;
    let snapshot = read_snapshot(&source, options)?;
    check_origin(&source, &target, options, options.resume)?;
    forge_required(
        &source,
        options,
        &["auth", "status", "--hostname", &target.host],
        "forge authentication failed; log in with the forge CLI first",
    )?;
    let protocol = protocol(&source, &target, options)?;
    let metadata = read_metadata(&source, &target, options)?;
    if !options.resume && metadata.is_some() {
        return Err("remote repository already exists; verify it and configure origin before using --resume".into());
    }
    if options.resume && metadata.is_none() {
        return Err("cannot verify existing remote; check forge access before resuming".into());
    }
    let resume_url = metadata
        .as_ref()
        .map(|value| verified_url(value, &target, options, &protocol))
        .transpose()?;
    if options.dry_run {
        if !options.resume {
            eprintln!(
                "repot: forge authentication checked; remote availability is unconfirmed until creation succeeds"
            );
        }
        println!(
            "would {} {} repository {} and push branch {} without force; then move {} to {}",
            if options.resume {
                "verify existing"
            } else {
                "attempt to create"
            },
            options.visibility.name(),
            target.url,
            snapshot.branch,
            source.display(),
            target.destination.display()
        );
        return Ok(0);
    }
    let url = if let Some(url) = resume_url {
        url
    } else {
        create(&source, &target, &snapshot, options)?;
        let value = read_metadata(&source, &target, options)?.ok_or(
            "remote created but verification failed; original checkout retained; verify the remote, configure origin and rerun with --resume",
        )?;
        let url = verified_url(&value, &target, options, &protocol)?;
        git_required(&source, options, &["remote", "add", "origin", &url])?;
        url
    };
    // Verify the remote metadata even when an existing origin uses the other
    // supported transport. Its identity, rather than protocol, must agree.
    if !matches_target(&url, &target, options) {
        return Err("forge returned an inconsistent repository URL".into());
    }
    check_origin(&source, &target, options, true)?;
    if read_snapshot(&source, options)? != snapshot {
        return Err("checkout changed during publication; review it before using --resume".into());
    }
    push(&source, &snapshot, options)?;
    if read_snapshot(&source, options)? != snapshot {
        return Err(
            "checkout changed after push; published commit verified but checkout retained".into(),
        );
    }
    git_required(
        &source,
        options,
        &[
            "config",
            "--local",
            &format!("branch.{}.remote", snapshot.branch),
            "origin",
        ],
    )?;
    git_required(
        &source,
        options,
        &[
            "config",
            "--local",
            &format!("branch.{}.merge", snapshot.branch),
            &format!("refs/heads/{}", snapshot.branch),
        ],
    )?;
    lifecycle::move_checkout(&source, &target.destination)?;
    eprintln!(
        "published {} and moved checkout to {}",
        target.url,
        target.destination.display()
    );
    navigation::handoff(&target.destination)?;
    Ok(0)
}

fn target(config: &Config, options: &Options) -> Result<Target> {
    let host = options
        .host
        .as_deref()
        .unwrap_or_else(|| options.forge.host())
        .to_ascii_lowercase();
    let url = format!("https://{host}/{}", options.repo);
    let (parsed_host, parts) = remote_parts(&url)?;
    if parsed_host != host
        || parts.join("/") != options.repo
        || (matches!(options.forge, Forge::Github) && parts.len() != 2)
    {
        return Err("publish requires a hostname and explicit owner/name (GitLab also supports nested groups)".into());
    }
    Ok(Target {
        host,
        destination: config.destination(&url)?,
        url,
    })
}

fn read_snapshot(path: &Path, options: &Options) -> Result<Snapshot> {
    let branch = git_required(
        path,
        options,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
    )
    .map_err(|_| "publish requires a named branch, not detached HEAD".to_owned())?;
    let head = git_required(path, options, &["rev-parse", "--verify", "HEAD"]).map_err(|_| {
        "publish requires an existing commit; commit your work manually first".to_owned()
    })?;
    if !git_required(
        path,
        options,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ],
    )?
    .is_empty()
    {
        return Err(
            "publish requires a clean working tree; commit or handle changes manually".into(),
        );
    }
    if git_required(path, options, &["ls-files", "-v", "-z"])?
        .split('\0')
        .any(|entry| {
            entry
                .chars()
                .next()
                .is_some_and(|flag| flag == 'S' || flag.is_ascii_lowercase())
        })
    {
        return Err(
            "publish requires index entries without assume-unchanged or skip-worktree flags".into(),
        );
    }
    Ok(Snapshot { branch, head })
}

fn check_origin(path: &Path, target: &Target, options: &Options, resume: bool) -> Result<()> {
    let remotes = git_required(path, options, &["remote"])?;
    if !resume {
        if remotes.is_empty() {
            return Ok(());
        }
        return Err(
            "checkout already has a remote; use --resume only for a verified matching origin"
                .into(),
        );
    }
    if remotes != "origin" {
        return Err("--resume requires exactly one remote named origin".into());
    }
    let urls = git_required(path, options, &["config", "--get-all", "remote.origin.url"])?;
    if urls.lines().count() != 1 || !matches_target(&urls, target, options) {
        return Err("origin does not match the requested publication target".into());
    }
    let push_urls = call(
        "git",
        path,
        options,
        &["config", "--get-all", "remote.origin.pushurl"],
    )?;
    if push_urls.success
        && (push_urls.stdout.lines().count() != 1
            || !matches_target(push_urls.stdout.trim_end_matches('\n'), target, options))
    {
        return Err("origin push URL does not match the requested publication target".into());
    }
    Ok(())
}

fn matches_target(url: &str, target: &Target, options: &Options) -> bool {
    remote_parts(url).is_ok_and(|(host, parts)| {
        host == target.host
            && match options.forge {
                Forge::Github => parts.join("/").eq_ignore_ascii_case(&options.repo),
                Forge::Gitlab => parts.join("/") == options.repo,
            }
    })
}

fn protocol(path: &Path, target: &Target, options: &Options) -> Result<String> {
    let output = call(
        options.forge.command(),
        path,
        options,
        &["config", "get", "git_protocol", "--host", &target.host],
    )?;
    if !output.success || output.stdout.trim().is_empty() {
        return Ok("https".into());
    }
    let protocol = output.stdout.trim();
    if !matches!(protocol, "ssh" | "https") {
        return Err("forge git_protocol must be ssh or https".into());
    }
    Ok(protocol.into())
}

fn read_metadata(path: &Path, target: &Target, options: &Options) -> Result<Option<Value>> {
    let args = match options.forge {
        Forge::Github => vec![
            "repo",
            "view",
            &target.url,
            "--json",
            "nameWithOwner,url,sshUrl,visibility",
        ],
        Forge::Gitlab => vec!["repo", "view", &target.url, "--output", "json"],
    };
    let output = call(options.forge.command(), path, options, &args)?;
    if !output.success {
        return Ok(None);
    }
    serde_json::from_str(&output.stdout)
        .map(Some)
        .map_err(|_| "forge returned invalid repository metadata".into())
}

fn verified_url(
    value: &Value,
    target: &Target,
    options: &Options,
    protocol: &str,
) -> Result<String> {
    let identity_field = match options.forge {
        Forge::Github => "nameWithOwner",
        Forge::Gitlab => "path_with_namespace",
    };
    let identity = value
        .get(identity_field)
        .and_then(Value::as_str)
        .ok_or("forge metadata lacks repository identity")?;
    let matches_identity = match options.forge {
        Forge::Github => identity.eq_ignore_ascii_case(&options.repo),
        Forge::Gitlab => identity == options.repo,
    };
    if !matches_identity {
        return Err("forge metadata identifies a different repository".into());
    }
    let visibility = value
        .get("visibility")
        .and_then(Value::as_str)
        .ok_or("forge metadata does not include visibility")?;
    if !visibility.eq_ignore_ascii_case(options.visibility.name()) {
        return Err(
            "remote visibility differs from --visibility; refusing to push or change visibility"
                .into(),
        );
    }
    let web_field = match options.forge {
        Forge::Github => "url",
        Forge::Gitlab => "http_url_to_repo",
    };
    let web = value
        .get(web_field)
        .and_then(Value::as_str)
        .ok_or("forge metadata lacks repository URL")?;
    if !matches_target(web, target, options) {
        return Err("forge metadata identifies a different repository".into());
    }
    let url = if protocol == "ssh" {
        let field = match options.forge {
            Forge::Github => "sshUrl",
            Forge::Gitlab => "ssh_url_to_repo",
        };
        value
            .get(field)
            .and_then(Value::as_str)
            .ok_or("forge metadata lacks SSH URL")?
    } else {
        web
    };
    if !matches_target(url, target, options) {
        return Err("forge metadata contains an inconsistent Git URL".into());
    }
    Ok(url.into())
}

fn create(path: &Path, target: &Target, snapshot: &Snapshot, options: &Options) -> Result<()> {
    let mut args = vec!["repo", "create", &target.url, options.visibility.flag()];
    if matches!(options.forge, Forge::Gitlab) {
        args.extend(["--skipGitInit", "--defaultBranch", &snapshot.branch]);
    }
    forge_required(
        path,
        options,
        &args,
        "forge creation failed; checkout retained; if the remote was created, verify it, configure origin and rerun with --resume",
    )?;
    Ok(())
}

fn push(path: &Path, snapshot: &Snapshot, options: &Options) -> Result<()> {
    let reference = format!("refs/heads/{}", snapshot.branch);
    let refspec = format!("{}:{reference}", snapshot.head);
    git_required(
        path,
        options,
        &[
            "-c",
            "remote.origin.mirror=false",
            "-c",
            "push.followTags=false",
            "push",
            "--porcelain",
            "--no-follow-tags",
            "origin",
            &refspec,
        ],
    )
    .map_err(|_| {
        "push failed; checkout and origin retained; inspect the remote then rerun with --resume"
            .to_owned()
    })?;
    let remote = git_required(
        path,
        options,
        &["ls-remote", "--exit-code", "origin", &reference],
    )?;
    let mut fields = remote.split_whitespace();
    if fields.next() != Some(snapshot.head.as_str())
        || fields.next() != Some(reference.as_str())
        || fields.next().is_some()
    {
        return Err(
            "remote branch tip does not match the inspected commit; checkout retained".into(),
        );
    }
    Ok(())
}

fn call(program: &str, path: &Path, options: &Options, args: &[&str]) -> Result<process::Output> {
    let args: Vec<_> = args.iter().map(OsStr::new).collect();
    process::run(program, &args, path, Duration::from_secs(options.timeout))
}

fn git_required(path: &Path, options: &Options, args: &[&str]) -> Result<String> {
    let output = call("git", path, options, args)?;
    if !output.success {
        return Err("Git publication check failed; inspect the checkout with git".into());
    }
    Ok(output
        .stdout
        .strip_suffix('\n')
        .unwrap_or(&output.stdout)
        .into())
}

fn forge_required(path: &Path, options: &Options, args: &[&str], error: &str) -> Result<String> {
    let output = call(options.forge.command(), path, options, args)?;
    if !output.success {
        return Err(error.into());
    }
    Ok(output.stdout)
}
