//! Open a repository in your editor or on its forge's web page.

use std::env;
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use crate::config::remote_parts;
use crate::{Result, process, ui};

#[derive(Debug, clap::Args)]
pub struct Options {
    /// Part of a repository name; defaults to the checkout you are in, or
    /// opens the picker.
    pub query: Option<String>,
    /// Open the repository's page on its forge instead of your editor.
    #[arg(long, short = 'w')]
    pub web: bool,
}

/// `REPOT_EDITOR`, then `VISUAL`, then `EDITOR`, split into program and flags.
fn editor() -> Option<Vec<OsString>> {
    ["REPOT_EDITOR", "VISUAL", "EDITOR"]
        .into_iter()
        .filter_map(env::var_os)
        .find(|value| !value.is_empty())
        .map(|value| {
            value
                .to_string_lossy()
                .split_whitespace()
                .map(OsString::from)
                .collect::<Vec<_>>()
        })
        .filter(|words| !words.is_empty())
}

/// The web page of a checkout's remote, on the current branch when it is not
/// the default one. Credentials in the remote URL never reach the page URL.
pub fn web_url(path: &Path) -> Result<String> {
    let remotes = process::git_optional(path, &["remote"])?
        .ok_or("this checkout has no remote; publish it with repot publish")?;
    let remote = if remotes.lines().any(|remote| remote == "origin") {
        "origin"
    } else {
        remotes
            .lines()
            .next()
            .ok_or("this checkout has no remote; publish it with repot publish")?
    };
    let url = process::git(path, &["remote", "get-url", "--", remote])?;
    // Tokens embedded in HTTPS remotes are dropped before anything else.
    let (host, parts) = remote_parts(&crate::info::public_url(&url))
        .map_err(|_| "the remote is not a web-hosted repository; open it in your editor instead")?;
    if url.starts_with("codecommit:") {
        return Err("CodeCommit repositories have no stable web page".into());
    }
    let mut page = format!("https://{host}/{}", parts.join("/"));
    let branch = process::git_optional(path, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    let default = process::git_optional(
        path,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            &format!("refs/remotes/{remote}/HEAD"),
        ],
    )?
    .and_then(|reference| {
        reference
            .strip_prefix(&format!("{remote}/"))
            .map(str::to_owned)
    });
    if let Some(branch) = branch
        && default.is_some_and(|default| default != branch)
    {
        let separator = if host.contains("gitlab") {
            "/-/tree/"
        } else {
            "/tree/"
        };
        page.push_str(separator);
        page.push_str(&branch);
    }
    Ok(page)
}

fn browser() -> Vec<OsString> {
    if let Some(browser) = env::var_os("BROWSER").filter(|value| !value.is_empty()) {
        return browser
            .to_string_lossy()
            .split_whitespace()
            .map(OsString::from)
            .collect();
    }
    vec![OsString::from(if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    })]
}

fn launch(words: &[OsString], argument: &OsString) -> Result<()> {
    let (program, flags) = words.split_first().ok_or("no program to launch")?;
    let status = Command::new(program)
        .args(flags)
        .arg(argument)
        .status()
        .map_err(|error| format!("cannot start {}: {error}", program.to_string_lossy()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} exited with {status}",
            program.to_string_lossy()
        ))
    }
}

pub fn run(path: &Path, roots: &[std::path::PathBuf], web: bool) -> Result<u8> {
    let name = ui::repository_name(path, roots);
    if web {
        let page = web_url(path)?;
        ui::note("↗", ui::INFO, &format!("opening {page}"));
        launch(&browser(), &OsString::from(page))?;
    } else {
        let editor = editor()
            .ok_or("no editor configured; set REPOT_EDITOR, VISUAL or EDITOR, or use --web")?;
        ui::note("↗", ui::INFO, &format!("opening {name}"));
        launch(&editor, &path.as_os_str().to_owned())?;
    }
    Ok(0)
}
