//! Bounded resolution of Git `CodeCommit` and Go vanity import URLs.

use std::ffi::OsStr;
use std::time::Duration;

use crate::{Result, config, process};

pub struct CodeCommit {
    pub url: String,
    pub region: String,
    pub repository: String,
}

/// The helper URL contains an optional profile name, never embedded credentials.
pub fn codecommit(input: &str) -> Option<Result<CodeCommit>> {
    input
        .starts_with("codecommit:")
        .then(|| resolve_codecommit(input))
}

fn safe_name(value: &str) -> bool {
    !value.is_empty()
        && !matches!(value, "." | "..")
        && !value.starts_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

fn resolve_codecommit(input: &str) -> Result<CodeCommit> {
    let invalid = "invalid CodeCommit repository, profile or region";
    let (explicit, rest) = if let Some(rest) = input.strip_prefix("codecommit://") {
        (None, rest)
    } else if let Some(rest) = input.strip_prefix("codecommit::") {
        let (region, rest) = rest.split_once("://").ok_or(invalid)?;
        (Some(region), rest)
    } else {
        return Err(invalid.into());
    };
    let (profile, repository) = rest
        .split_once('@')
        .map_or((None, rest), |(profile, repository)| {
            (Some(profile), repository)
        });
    if !safe_name(repository) || profile.is_some_and(|profile| !safe_name(profile)) {
        return Err(invalid.into());
    }
    let region = explicit
        .map(str::to_owned)
        .or_else(|| std::env::var("AWS_REGION").ok())
        .or_else(|| std::env::var("AWS_DEFAULT_REGION").ok())
        .map_or_else(|| aws_region(profile), Ok)?;
    if !region
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase())
        || !region
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(invalid.into());
    }
    let identity = profile.map_or_else(
        || repository.to_owned(),
        |profile| format!("{profile}@{repository}"),
    );
    Ok(CodeCommit {
        url: format!("codecommit::{region}://{identity}"),
        region,
        repository: repository.into(),
    })
}

fn aws_region(profile: Option<&str>) -> Result<String> {
    let mut args = vec!["configure", "get", "region"];
    if let Some(profile) = profile {
        args.extend(["--profile", profile]);
    }
    let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    let cwd = std::env::current_dir().map_err(|_| "cannot resolve current directory")?;
    let output = process::run("aws", &args, &cwd, Duration::from_secs(5))
        .map_err(|_| "set a CodeCommit region explicitly or configure an AWS region")?;
    if !output.success {
        return Err("set a CodeCommit region explicitly or configure an AWS region".into());
    }
    Ok(output.stdout.trim().into())
}

#[derive(Debug, PartialEq, Eq)]
pub struct Vanity {
    pub url: String,
    pub prefix: String,
}

/// Read at most 1 MiB, without redirects, cookies or credentials.
/// The caller invokes this only after a normal HTTP(S) Git clone fails.
pub fn vanity(input: &str, timeout: Duration) -> Result<Option<Vanity>> {
    let Some(import_path) = http_import(input)? else {
        return Ok(None);
    };
    let started = std::time::Instant::now();
    let found = request_metadata(input, import_path, timeout)?;
    if let Some(mapping) = &found
        && mapping.prefix != import_path
    {
        let scheme = if input.starts_with("https://") {
            "https"
        } else {
            "http"
        };
        let prefix_url = format!("{scheme}://{}", mapping.prefix);
        let remaining = timeout
            .checked_sub(started.elapsed())
            .ok_or("vanity metadata timed out")?;
        if request_metadata(&prefix_url, &mapping.prefix, remaining)?.as_ref() != Some(mapping) {
            return Err("vanity metadata prefix verification failed".into());
        }
    }
    Ok(found)
}

fn request_metadata(input: &str, import_path: &str, timeout: Duration) -> Result<Option<Vanity>> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .max_redirects(0)
        .build()
        .into();
    let mut response = agent
        .get(format!("{input}?go-get=1"))
        .header("User-Agent", concat!("repot/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|_| "vanity metadata request failed")?;
    if !response.status().is_success() {
        return Err("vanity metadata request failed".into());
    }
    let html = response
        .body_mut()
        .with_config()
        .limit(1024 * 1024)
        .read_to_string()
        .map_err(|_| "vanity metadata exceeds size limit or could not be read")?;
    parse_metadata(&html, import_path)
}

fn http_import(input: &str) -> Result<Option<&str>> {
    let Some(import) = input
        .strip_prefix("https://")
        .or_else(|| input.strip_prefix("http://"))
    else {
        return Ok(None);
    };
    config::remote_parts(input)?;
    Ok(Some(import.trim_end_matches('/')))
}

fn parse_metadata(html: &str, import: &str) -> Result<Option<Vanity>> {
    let mut found = None;
    let mut emitter = html5gum::DefaultEmitter::default();
    emitter.naively_switch_states(true);
    for token in html5gum::Tokenizer::new_with_emitter(html, emitter).flatten() {
        if matches!(&token, html5gum::Token::EndTag(tag) if tag.name.as_slice() == b"head") {
            break;
        }
        let html5gum::Token::StartTag(tag) = token else {
            continue;
        };
        if tag.name.as_slice() == b"body" {
            break;
        }
        if tag.name.as_slice() != b"meta"
            || tag
                .attributes
                .get(b"name".as_slice())
                .is_none_or(|value| value.as_slice() != b"go-import")
        {
            continue;
        }
        let Some(content) = tag.attributes.get(b"content".as_slice()) else {
            continue;
        };
        let content = std::str::from_utf8(content).map_err(|_| "vanity metadata is not UTF-8")?;
        let mut fields = content.split_whitespace();
        let (Some(prefix), Some(vcs), Some(url), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if vcs != "git"
            || !(import == prefix
                || import
                    .strip_prefix(prefix)
                    .is_some_and(|suffix| suffix.starts_with('/')))
        {
            continue;
        }
        config::remote_parts(&format!("https://{prefix}"))?;
        if !["https://", "http://", "ssh://", "git://"]
            .iter()
            .any(|scheme| url.starts_with(scheme))
        {
            return Err("vanity metadata contains an unsupported Git URL".into());
        }
        config::remote_parts(url)?;
        let candidate = Vanity {
            url: url.into(),
            prefix: prefix.into(),
        };
        if found
            .as_ref()
            .is_some_and(|previous| previous != &candidate)
        {
            return Err("vanity metadata contains ambiguous Git mappings".into());
        }
        found = Some(candidate);
    }
    Ok(found)
}
