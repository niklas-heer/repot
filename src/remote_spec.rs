//! Resolve familiar ghq repository specifications without leaking credentials.

use std::path::PathBuf;

use crate::Result;
use crate::config::{self, Config};

#[derive(Clone, Debug)]
pub struct Spec {
    pub url: String,
    pub path: PathBuf,
    pub branch: Option<String>,
}

pub fn resolve(config: &Config, input: &str, ssh: bool, force_user: bool) -> Result<Spec> {
    let input = input.trim_end_matches('/');
    if input.is_empty() || input.chars().any(char::is_whitespace) || input.starts_with('-') {
        return Err("invalid repository specification".into());
    }
    if let Some(codecommit) = crate::remote_extra::codecommit(input) {
        let codecommit = codecommit?;
        let path = config.destination(&codecommit.url)?;
        return Ok(Spec {
            url: codecommit.url,
            path,
            branch: None,
        });
    }
    let (input, branch) = split_branch(input);
    let expanded = relative(config, input)?;
    let input = expanded.as_deref().unwrap_or(input);
    let mut url = if input.contains("://") || input.contains(':') {
        input.to_owned()
    } else {
        let first = input.split('/').next().unwrap_or_default();
        if input.contains('/') && (first.contains('.') || first == "localhost") {
            format!("https://{input}")
        } else {
            let host = config::git_value("ghq.defaultHost")?
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "github.com".into());
            let path = if input.contains('/') {
                input.to_owned()
            } else {
                let same = !force_user && config::git_bool("ghq.completeUser")? == Some(false);
                let owner = if same { input.to_owned() } else { username()? };
                format!("{owner}/{input}")
            };
            format!("https://{host}/{path}")
        }
    };
    if !url.contains("://")
        && let Some((authority, path)) = url.split_once(':')
    {
        url = format!("ssh://{authority}/{}", path.trim_start_matches('/'));
    }
    let (host, parts) = config::remote_parts(&url)?;
    if host == "github.com" && parts.len() > 2 {
        let (scheme, rest) = url.split_once("://").ok_or("invalid repository URL")?;
        let mut segments = rest.split('/');
        let authority = segments.next().ok_or("missing GitHub authority")?;
        let owner = segments.next().ok_or("missing GitHub owner")?;
        let repository = segments.next().ok_or("missing GitHub repository")?;
        url = format!("{scheme}://{authority}/{owner}/{repository}");
    }
    if ssh
        && (url.starts_with("https://") || url.starts_with("http://") || url.starts_with("git://"))
    {
        let (_, rest) = url.split_once("://").ok_or("invalid repository URL")?;
        url = format!("ssh://git@{rest}");
    }
    let path = config.destination(&url)?;
    Ok(Spec { url, path, branch })
}

fn split_branch(input: &str) -> (&str, Option<String>) {
    input
        .rsplit_once('@')
        .map_or((input, None), |(url, branch)| {
            let suffix = url
                .split_once("://")
                .map_or_else(|| !branch.contains(':'), |(_, rest)| rest.contains('/'));
            if suffix {
                (url, Some(branch.to_owned()))
            } else {
                (input, None)
            }
        })
}

fn username() -> Result<String> {
    for key in ["ghq.user", "github.user"] {
        if let Some(value) = config::git_value(key)?.filter(|value| !value.is_empty()) {
            return Ok(value);
        }
    }
    std::env::var("USER")
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "set ghq.user in Git configuration for a repository without an owner".into())
}

fn relative(config: &Config, input: &str) -> Result<Option<String>> {
    if !input.starts_with("./") && !input.starts_with("../") {
        return Ok(None);
    }
    let mut absolute = std::env::current_dir().map_err(|error| error.to_string())?;
    for part in std::path::Path::new(input).components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                absolute.pop();
            }
            part => absolute.push(part.as_os_str()),
        }
    }
    let relative = config
        .roots
        .iter()
        .filter_map(|root| absolute.strip_prefix(root).ok())
        .min_by_key(|path| path.components().count())
        .ok_or("relative repository specifications must stay inside a configured root")?;
    Ok(Some(format!("https://{}", relative.to_string_lossy())))
}

pub fn git_vcs(value: Option<&str>) -> Result<()> {
    if value.is_some_and(|value| !matches!(value, "git" | "github" | "codecommit")) {
        return Err(
            "repot supports Git repositories; --vcs accepts git, github or codecommit".into(),
        );
    }
    Ok(())
}
