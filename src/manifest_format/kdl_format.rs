//! A strict KDL v2 projection of the shared manifest model.

use kdl::{KdlDocument, KdlEntry, KdlNode, KdlNodeFormat, KdlValue};

use crate::Result;
use crate::config::{Manifest, RepoEntry, Settings};

const INVALID: &str = "invalid KDL manifest schema";

pub fn parse(text: &str) -> Result<Manifest> {
    let document = document(text)?;
    let mut manifest = Manifest::default();
    let mut settings_seen = false;
    for node in document.nodes() {
        if node.ty().is_some() {
            return Err(INVALID.into());
        }
        match node.name().value() {
            "settings" if !settings_seen => {
                settings_seen = true;
                manifest.settings = settings(node)?;
            }
            "repo" => manifest.repo.push(repository(node)?),
            _ => return Err(INVALID.into()),
        }
    }
    Ok(manifest)
}

fn document(text: &str) -> Result<KdlDocument> {
    text.parse()
        .map_err(|_| "invalid KDL manifest syntax".into())
}

fn settings(node: &KdlNode) -> Result<Settings> {
    if !node.entries().is_empty() {
        return Err(INVALID.into());
    }
    let mut result = Settings::default();
    let mut owners_seen = false;
    if let Some(children) = node.children() {
        for child in children.nodes() {
            if child.name().value() != "owners"
                || owners_seen
                || child.ty().is_some()
                || child.children().is_some()
            {
                return Err(INVALID.into());
            }
            owners_seen = true;
            for owner in child.entries() {
                if owner.name().is_some() || owner.ty().is_some() {
                    return Err(INVALID.into());
                }
                result.owners.push(string(owner)?);
            }
        }
    }
    Ok(result)
}

fn repository(node: &KdlNode) -> Result<RepoEntry> {
    if node.children().is_some() {
        return Err(INVALID.into());
    }
    let mut url = None;
    let mut path = None;
    let mut restore = None;
    for entry in node.entries() {
        if entry.ty().is_some() {
            return Err(INVALID.into());
        }
        match entry.name().map(kdl::KdlIdentifier::value) {
            None if url.is_none() => url = Some(string(entry)?),
            Some("path") if path.is_none() => path = Some(string(entry)?),
            Some("restore") if restore.is_none() => {
                restore = Some(entry.value().as_bool().ok_or(INVALID)?);
            }
            _ => return Err(INVALID.into()),
        }
    }
    Ok(RepoEntry {
        url: url.ok_or(INVALID)?,
        path,
        restore: restore.unwrap_or(true),
    })
}

fn string(entry: &KdlEntry) -> Result<String> {
    entry
        .value()
        .as_string()
        .map(str::to_owned)
        .ok_or_else(|| INVALID.into())
}

pub fn edit(original: &str, matching: Option<usize>, entry: &RepoEntry) -> Result<String> {
    let mut document = document(original)?;
    if let Some(index) = matching {
        let node = document
            .nodes_mut()
            .iter_mut()
            .filter(|node| node.name().value() == "repo")
            .nth(index)
            .ok_or("manifest entry changed")?;
        set_value(node, None, KdlValue::String(entry.url.clone()));
        if let Some(path) = &entry.path {
            set_value(node, Some("path"), KdlValue::String(path.clone()));
        } else {
            remove_path(node);
        }
        if !entry.restore {
            set_value(node, Some("restore"), KdlValue::Bool(false));
        }
    } else {
        let mut node = KdlNode::new("repo");
        node.push(entry.url.clone());
        if let Some(path) = &entry.path {
            node.push(("path", path.clone()));
        }
        node.push(("restore", entry.restore));
        node.set_format(KdlNodeFormat {
            leading: if original.is_empty() {
                String::new()
            } else {
                "\n".into()
            },
            terminator: "\n".into(),
            ..KdlNodeFormat::default()
        });
        document.nodes_mut().push(node);
    }
    let rendered = document.to_string();
    // Formatting edits must never silently change or invalidate the schema.
    parse(&rendered)?;
    Ok(rendered)
}

fn set_value(node: &mut KdlNode, name: Option<&str>, value: KdlValue) {
    if let Some(entry) = node
        .entries_mut()
        .iter_mut()
        .find(|entry| entry.name().map(kdl::KdlIdentifier::value) == name)
    {
        if entry.value() != &value {
            if let Some(format) = entry.format_mut() {
                format.value_repr = value.to_string();
            }
            entry.set_value(value);
        }
    } else {
        let mut entry = KdlEntry::new(value);
        if let Some(name) = name {
            entry.set_name(Some(name));
        }
        node.push(entry);
    }
}

fn remove_path(node: &mut KdlNode) {
    let mut comments = String::new();
    node.entries_mut().retain(|entry| {
        if entry.name().is_none_or(|name| name.value() != "path") {
            return true;
        }
        if let Some(format) = entry.format() {
            for trivia in [
                &format.leading,
                &format.after_key,
                &format.after_eq,
                &format.trailing,
            ] {
                comments.push_str(trivia);
            }
        }
        false
    });
    if let Some(format) = node.format_mut() {
        comments.push_str(&format.before_terminator);
        format.before_terminator = comments;
    }
}
