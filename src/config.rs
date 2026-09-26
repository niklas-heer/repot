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
        let manifest_path = manifest.map_or_else(
            || {
                env::var_os("XDG_CONFIG_HOME")
                    .map_or_else(|| home.join(".config"), PathBuf::from)
                    .join("repot/repos.toml")
            },
            Path::to_path_buf,
        );
        let manifest_path = if manifest_path.is_absolute() {
            manifest_path
        } else {
            cwd.join(manifest_path)
        };
        let manifest = match fs::read_to_string(&manifest_path) {
            Ok(text) => toml::from_str(&text)
                .map_err(|_| format!("invalid manifest at {}", manifest_path.display()))?,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && (manifest.is_none() || allow_missing) =>
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
        let (host, components) = remote_parts(url)?;
        let relative: PathBuf = std::iter::once(host).chain(components).collect();
        if let Some(existing) = self
            .roots
            .iter()
            .map(|root| root.join(&relative))
            .find(|path| path.exists())
        {
            return Ok(existing);
        }
        self.roots
            .last()
            .map(|root| root.join(relative))
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

fn home() -> Result<PathBuf> {
    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "HOME must name your home directory".to_owned())
}

fn roots(cwd: &Path, home: &Path) -> Result<Vec<PathBuf>> {
    if let Some(root) = env::var_os("GHQ_ROOT").filter(|value| !value.is_empty()) {
        return Ok(vec![expand_path(&root.to_string_lossy(), cwd, home)?]);
    }
    let output = Command::new("git")
        .args(["config", "--null", "--get-all", "ghq.root"])
        .output()
        .map_err(|error| format!("read ghq roots: {error}"))?;
    if output.status.code() == Some(1) {
        return Ok(vec![home.join("ghq")]);
    }
    if !output.status.success() {
        return Err("could not read Git configuration for ghq.root".to_owned());
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|_| "ghq.root contains a non-UTF-8 path".to_owned())?;
    let roots: Result<Vec<_>> = text
        .split('\0')
        .filter(|value| !value.is_empty())
        .map(|value| expand_path(value, cwd, home))
        .collect();
    let roots = roots?;
    if roots.is_empty() {
        Ok(vec![home.join("ghq")])
    } else {
        Ok(roots)
    }
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
    if components.len() < 2
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
