//! ghq-compatible roots and the portable repository manifest.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::Result;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default)]
    pub owners: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepoEntry {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default = "restore_default")]
    pub restore: bool,
}

const fn restore_default() -> bool {
    true
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub repo: Vec<RepoEntry>,
}

#[derive(Debug)]
pub struct Config {
    pub roots: Vec<PathBuf>,
    pub manifest_path: PathBuf,
    pub manifest: Manifest,
}

impl Config {
    pub fn load(manifest: Option<&Path>) -> Result<Self> {
        Self::load_inner(manifest, false)
    }

    pub fn load_for_write(manifest: Option<&Path>) -> Result<Self> {
        Self::load_inner(manifest, true)
    }

    fn load_inner(manifest: Option<&Path>, allow_missing: bool) -> Result<Self> {
        let cwd = env::current_dir().map_err(|error| format!("read current directory: {error}"))?;
        let home = home()?;
        let manifest_path =
            manifest.map_or_else(|| default_manifest(&home), |path| Ok(path.to_path_buf()))?;
        let manifest_path = if manifest_path.is_absolute() {
            manifest_path
        } else {
            cwd.join(manifest_path)
        };
        let manifest = match fs::read_to_string(&manifest_path) {
            Ok(text) => crate::manifest_format::parse(&manifest_path, &text)
                .map_err(|_| format!("invalid manifest at {}", manifest_path.display()))?,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && (manifest.is_none() || allow_missing)
                    && !manifest_exists(&manifest_path)? =>
            {
                Manifest::default()
            }
            Err(error) => {
                return Err(format!(
                    "read manifest {}: {error}",
                    manifest_path.display()
                ));
            }
        };
        Ok(Self {
            roots: roots(&cwd, &home)?,
            manifest_path,
            manifest,
        })
    }

    pub fn destination(&self, url: &str) -> Result<PathBuf> {
        self.destination_kind(url, false)
    }

    pub fn destination_bare(&self, url: &str) -> Result<PathBuf> {
        self.destination_kind(url, true)
    }

    fn destination_kind(&self, url: &str, bare: bool) -> Result<PathBuf> {
        let (host, components) = remote_parts(url)?;
        let mut relative: PathBuf = std::iter::once(host).chain(components).collect();
        if bare {
            let mut filename = relative
                .file_name()
                .ok_or("repository path has no name")?
                .to_owned();
            filename.push(".git");
            relative.set_file_name(filename);
        }
        if let Some(existing) = self
            .roots
            .iter()
            .rev()
            .map(|root| root.join(&relative))
            .find(|path| path.exists())
        {
            return public_destination(existing);
        }
        let lookup_url = if url.contains("://") {
            url.to_owned()
        } else if let Some((authority, path)) = url.split_once(':') {
            format!("ssh://{authority}/{}", path.trim_start_matches('/'))
        } else {
            format!("https://{url}")
        };
        if !url.starts_with("codecommit:")
            && env::var_os("GHQ_ROOT").is_none_or(|value| value.is_empty())
            && let Some(root) = git_config(&["--path", "--get-urlmatch", "ghq.root", &lookup_url])?
        {
            let cwd = env::current_dir().map_err(|error| error.to_string())?;
            return public_destination(
                expand_path(root.trim_end_matches('\n'), &cwd, &home()?)?.join(relative),
            );
        }
        public_destination(self.primary_root()?.join(relative))
    }

    pub fn primary_root(&self) -> Result<&Path> {
        self.roots
            .last()
            .map(PathBuf::as_path)
            .ok_or_else(|| "no repository root configured".to_owned())
    }

    pub fn expand_path(&self, path: &str) -> Result<PathBuf> {
        let base = self
            .manifest_path
            .parent()
            .ok_or_else(|| "manifest has no parent directory".to_owned())?;
        expand_path(path, base, &home()?)
    }
}

fn default_manifest(home: &Path) -> Result<PathBuf> {
    let directory = env::var_os("XDG_CONFIG_HOME")
        .map_or_else(|| home.join(".config"), PathBuf::from)
        .join("repot");
    let toml = directory.join("repos.toml");
    let kdl = directory.join("repos.kdl");
    match (manifest_exists(&toml)?, manifest_exists(&kdl)?) {
        (true, true) => {
            Err("both repos.toml and repos.kdl exist; choose one with --manifest".into())
        }
        (false, true) => Ok(kdl),
        _ => Ok(toml),
    }
}

fn manifest_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("inspect manifest {}: {error}", path.display())),
    }
}

fn public_destination(path: PathBuf) -> Result<PathBuf> {
    crate::discovery::validate_public_path(&path)?;
    Ok(path)
}

fn home() -> Result<PathBuf> {
    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "HOME must name your home directory".to_owned())
}

fn roots(cwd: &Path, home: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = if let Some(roots) = env::var_os("GHQ_ROOT").filter(|value| !value.is_empty()) {
        env::split_paths(&roots)
            .map(|path| {
                let path = if path.as_os_str().is_empty() {
                    Path::new(".")
                } else {
                    &path
                };
                expand_path(&path.to_string_lossy(), cwd, home)
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        let configured = git_config(&["--null", "--path", "--get-regexp", "^ghq\\.(.*\\.)?root$"])?;
        let mut values = Vec::new();
        let mut scoped = Vec::new();
        for item in configured
            .as_deref()
            .unwrap_or_default()
            .split('\0')
            .filter(|value| !value.is_empty())
        {
            let (key, value) = item.split_once('\n').ok_or("invalid root configuration")?;
            let path = expand_path(value, cwd, home)?;
            if key == "ghq.root" {
                values.push(path);
            } else {
                scoped.push(path);
            }
        }
        values.reverse();
        if values.is_empty() {
            values.push(home.join("ghq"));
        }
        values.extend(scoped);
        values
    };
    let mut seen = std::collections::BTreeSet::new();
    for path in &mut paths {
        *path = path.canonicalize().unwrap_or_else(|_| normalize_path(path));
    }
    paths.retain(|path| seen.insert(path.clone()));
    // Existing repot callers use the last root as primary. Preserve that API
    // while representing ghq's complete ordering by reversing at this boundary.
    paths.reverse();
    Ok(paths)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                result.pop();
            }
            component => result.push(component.as_os_str()),
        }
    }
    result
}

pub fn git_value(key: &str) -> Result<Option<String>> {
    git_config(&["--get", key])
        .map(|value| value.map(|value| value.trim_end_matches('\n').to_owned()))
}

pub fn git_bool(key: &str) -> Result<Option<bool>> {
    git_config(&["--bool", "--get", key])
        .map(|value| value.map(|value| value.trim_end_matches('\n') == "true"))
}

pub fn git_url_value(key: &str, url: &str) -> Result<Option<String>> {
    if url.starts_with("codecommit:") {
        return Ok(None);
    }
    git_config(&["--get-urlmatch", key, url])
        .map(|value| value.map(|value| value.trim_end_matches('\n').to_owned()))
}

fn git_config(args: &[&str]) -> Result<Option<String>> {
    let output = Command::new("git")
        .arg("config")
        .args(args)
        .output()
        .map_err(|error| format!("read Git configuration: {error}"))?;
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    if !output.status.success() {
        return Err("could not read Git configuration".to_owned());
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|_| "Git configuration contains non-UTF-8 data".to_owned())?;
    Ok(Some(text))
}

fn expand_path(value: &str, base: &Path, home: &Path) -> Result<PathBuf> {
    if value.is_empty() || value.contains('\0') {
        return Err("repository path must not be empty or contain NUL".to_owned());
    }
    let mut expanded = String::new();
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        if character != '$' {
            expanded.push(character);
            continue;
        }
        if chars.next_if_eq(&'$').is_some() {
            expanded.push('$');
            continue;
        }
        let name = if chars.next_if_eq(&'{').is_some() {
            let name: String = chars
                .by_ref()
                .take_while(|character| *character != '}')
                .collect();
            name
        } else {
            let mut name = String::new();
            while let Some(character) = chars.next_if(|c| c.is_ascii_alphanumeric() || *c == '_') {
                name.push(character);
            }
            name
        };
        if name.is_empty() {
            expanded.push('$');
        } else {
            expanded
                .push_str(&env::var(&name).map_err(|_| format!("path variable {name} is unset"))?);
        }
    }
    let path = if expanded == "~" {
        home.to_path_buf()
    } else if let Some(relative) = expanded.strip_prefix("~/") {
        home.join(relative)
    } else {
        PathBuf::from(expanded)
    };
    Ok(if path.is_absolute() {
        path
    } else {
        base.join(path)
    })
}

/// Parse supported forge URLs without ever including a potentially secret URL in errors.
pub fn remote_parts(url: &str) -> Result<(String, Vec<String>)> {
    if let Some(codecommit) = crate::remote_extra::codecommit(url) {
        let codecommit = codecommit?;
        return Ok((codecommit.region, vec![codecommit.repository]));
    }
    let invalid =
        || "remote must be a credential-free HTTPS, SSH or host/owner/repository URL".to_owned();
    if url.is_empty()
        || url.starts_with('-')
        || url.chars().any(char::is_whitespace)
        || url.contains(['?', '#', '%', '\\'])
    {
        return Err(invalid());
    }
    let (authority, path, ssh) = if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .or_else(|| url.strip_prefix("git://"))
    {
        let (authority, path) = rest.split_once('/').ok_or_else(invalid)?;
        (authority, path, false)
    } else if let Some(rest) = url.strip_prefix("ssh://") {
        let (authority, path) = rest.split_once('/').ok_or_else(invalid)?;
        (authority, path, true)
    } else if let Some((authority, path)) = url.split_once(':') {
        (authority, path, true)
    } else {
        let (authority, path) = url.split_once('/').ok_or_else(invalid)?;
        (authority, path, false)
    };
    let authority = if ssh {
        if let Some((user, authority)) = authority.split_once('@') {
            if user.is_empty()
                || !user
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_-".contains(c))
            {
                return Err(invalid());
            }
            authority
        } else {
            authority
        }
    } else {
        authority
    };
    let host = if let Some((host, port)) = authority.split_once(':') {
        if port.parse::<u16>().is_err() {
            return Err(invalid());
        }
        host
    } else {
        authority
    };
    if host.is_empty()
        || host.starts_with(['-', '.'])
        || host.ends_with('.')
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c))
    {
        return Err(invalid());
    }
    let path = path.strip_suffix(".git").unwrap_or(path);
    let components: Vec<String> = path.split('/').map(str::to_owned).collect();
    if components.is_empty()
        || components.iter().any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || part.starts_with('-')
                || !part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        })
    {
        return Err(invalid());
    }
    Ok((host.to_ascii_lowercase(), components))
}
