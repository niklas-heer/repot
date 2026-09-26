//! A strict schema over YAML's lossless syntax tree; diagnostics never echo values.

use std::collections::HashSet;

use yaml_edit::{Mapping, Sequence, SyntaxKind, YamlFile, YamlNode};

use crate::Result;
use crate::config::{Manifest, RepoEntry};

const INVALID: &str = "invalid YAML manifest";

fn document(text: &str) -> Result<YamlFile> {
    // Indirection makes the effect of an edit nonlocal. Accept concrete values only.
    if yaml_edit::lex(text).iter().any(|(kind, _)| {
        matches!(
            kind,
            SyntaxKind::ANCHOR | SyntaxKind::REFERENCE | SyntaxKind::TAG
        )
    }) {
        return Err(INVALID.into());
    }
    let file = text.parse::<YamlFile>().map_err(|_| INVALID)?;
    if file.documents().count() > 1 || file.directives().next().is_some() {
        return Err(INVALID.into());
    }
    Ok(file)
}

pub fn parse(text: &str) -> Result<Manifest> {
    let file = document(text)?;
    let mut manifest = Manifest::default();
    let Some(document) = file.document() else {
        return Ok(manifest);
    };
    let mapping = document.as_mapping().ok_or(INVALID)?;
    for (key, value) in fields(&mapping, &["settings", "repo"])? {
        match key.as_str() {
            "settings" => {
                for (_, owners) in fields(value.as_mapping().ok_or(INVALID)?, &["owners"])? {
                    manifest.settings.owners = owners
                        .as_sequence()
                        .ok_or(INVALID)?
                        .values()
                        .map(|owner| string(&owner))
                        .collect::<Result<_>>()?;
                }
            }
            "repo" => {
                manifest.repo = value
                    .as_sequence()
                    .ok_or(INVALID)?
                    .values()
                    .map(|entry| repository(&entry))
                    .collect::<Result<_>>()?;
            }
            _ => return Err(INVALID.into()),
        }
    }
    Ok(manifest)
}

fn fields(mapping: &Mapping, allowed: &[&str]) -> Result<Vec<(String, YamlNode)>> {
    let mut seen = HashSet::new();
    mapping
        .entries()
        .map(|entry| {
            let key = string(&entry.key_node().ok_or(INVALID)?)?;
            if !allowed.contains(&key.as_str()) || !seen.insert(key.clone()) {
                return Err(INVALID.into());
            }
            Ok((key, entry.value_node().ok_or(INVALID)?))
        })
        .collect()
}

fn string(node: &YamlNode) -> Result<String> {
    let scalar = node.as_scalar().ok_or(INVALID)?;
    let raw = scalar.value();
    if !scalar.is_quoted() && !raw.starts_with(['|', '>']) && !plain_string(&raw) {
        return Err(INVALID.into());
    }
    Ok(scalar.as_string())
}

fn plain_string(raw: &str) -> bool {
    // Classify YAML 1.2 core values lexically: numeric range must never turn
    // an integer into a string, and an implicit empty scalar is null.
    if matches!(
        raw,
        "" | "null"
            | "Null"
            | "NULL"
            | "~"
            | "true"
            | "True"
            | "TRUE"
            | "false"
            | "False"
            | "FALSE"
            | ".inf"
            | ".Inf"
            | ".INF"
            | "+.inf"
            | "+.Inf"
            | "+.INF"
            | "-.inf"
            | "-.Inf"
            | "-.INF"
            | ".nan"
            | ".NaN"
            | ".NAN"
    ) {
        return false;
    }
    if raw.strip_prefix("0x").is_some_and(|digits| {
        !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) || raw.strip_prefix("0o").is_some_and(|digits| {
        !digits.is_empty() && digits.bytes().all(|byte| matches!(byte, b'0'..=b'7'))
    }) {
        return false;
    }
    !(raw.bytes().any(|byte| byte.is_ascii_digit()) && raw.parse::<f64>().is_ok())
}

fn repository(node: &YamlNode) -> Result<RepoEntry> {
    let mut url = None;
    let mut path = None;
    let mut restore = true;
    for (key, value) in fields(
        node.as_mapping().ok_or(INVALID)?,
        &["url", "path", "restore"],
    )? {
        match key.as_str() {
            "url" => url = Some(string(&value)?),
            "path" => path = Some(string(&value)?),
            "restore" => {
                restore = match value.as_scalar().ok_or(INVALID)?.value().as_str() {
                    "true" | "True" | "TRUE" => true,
                    "false" | "False" | "FALSE" => false,
                    _ => return Err(INVALID.into()),
                };
            }
            _ => return Err(INVALID.into()),
        }
    }
    Ok(RepoEntry {
        url: url.ok_or(INVALID)?,
        path,
        restore,
    })
}

pub fn edit(original: &str, matching: Option<usize>, entry: &RepoEntry) -> Result<String> {
    parse(original)?;
    // A terminating newline keeps appended nodes separate from a final comment.
    let mut source = original.to_owned();
    if !source.is_empty() && !source.ends_with('\n') {
        source.push('\n');
    }
    let file = document(&source)?;
    let document = file.ensure_document();
    if document.get("repo").is_none() {
        document.set("repo", Sequence::new_pending_block());
    }
    let repos_node = document.get("repo").ok_or(INVALID)?;
    let repos = repos_node.as_sequence().ok_or(INVALID)?;
    let mut removed_comments = String::new();
    if let Some(index) = matching {
        let node = repos.get(index).ok_or("manifest entry changed")?;
        let mapping = node.as_mapping().ok_or(INVALID)?;
        set_string(mapping, "url", &entry.url);
        if let Some(path) = &entry.path {
            set_string(mapping, "path", path);
        } else if let Some(removed) = mapping.remove("path") {
            for (kind, text) in yaml_edit::lex(&removed.to_string()) {
                if kind == SyntaxKind::COMMENT {
                    removed_comments.push_str(text);
                    removed_comments.push('\n');
                }
            }
        }
        if !entry.restore
            && !mapping.get("restore").is_some_and(|value| {
                value
                    .as_scalar()
                    .is_some_and(|scalar| scalar.as_bool() == Some(false))
            })
        {
            mapping.set("restore", false);
        }
    } else {
        // yaml-edit copies existing AST nodes without reindenting them. A flow
        // mapping remains valid inside either a block or a flow repo sequence.
        let mapping = Mapping::new_flow();
        mapping.set("url", entry.url.as_str());
        if let Some(path) = &entry.path {
            mapping.set("path", path.as_str());
        }
        mapping.set("restore", entry.restore);
        repos.push(mapping);
    }
    let rendered = format!("{removed_comments}{file}");
    parse(&rendered)?;
    Ok(rendered)
}

fn set_string(mapping: &Mapping, key: &str, value: &str) {
    if !mapping
        .get(key)
        .is_some_and(|node| string(&node).is_ok_and(|previous| previous == value))
    {
        mapping.set(key, value);
    }
}
