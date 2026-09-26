//! Format selection by logical path, with comment-preserving document edits.

use std::path::Path;

use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

use crate::Result;
use crate::config::{Manifest, RepoEntry};

mod kdl_format;

pub fn parse(path: &Path, text: &str) -> Result<Manifest> {
    if is_kdl(path) {
        kdl_format::parse(text)
    } else {
        toml::from_str(text).map_err(|_| "invalid TOML manifest".into())
    }
}

pub fn edit(
    path: &Path,
    original: &str,
    matching: Option<usize>,
    entry: &RepoEntry,
) -> Result<String> {
    if is_kdl(path) {
        kdl_format::edit(original, matching, entry)
    } else {
        edit_toml(original, matching, entry)
    }
}

fn is_kdl(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "kdl")
}

fn edit_toml(original: &str, matching: Option<usize>, entry: &RepoEntry) -> Result<String> {
    let mut document = original
        .parse::<DocumentMut>()
        .map_err(|_| "manifest formatting could not be parsed".to_owned())?;
    if document.get("repo").is_none() {
        document.insert("repo", Item::ArrayOfTables(ArrayOfTables::new()));
    }
    let repos = document
        .get_mut("repo")
        .and_then(Item::as_array_of_tables_mut)
        .ok_or("manifest editing requires [[repo]] entries")?;
    if let Some(index) = matching {
        let table = repos.get_mut(index).ok_or("manifest entry changed")?;
        set_preserving_comment(table, "url", value(&entry.url));
        if let Some(path) = &entry.path {
            set_preserving_comment(table, "path", value(path));
        } else {
            table.remove("path");
        }
        if !entry.restore {
            set_preserving_comment(table, "restore", value(false));
        }
    } else {
        let mut table = Table::new();
        table.insert("url", value(&entry.url));
        if let Some(path) = &entry.path {
            table.insert("path", value(path));
        }
        table.insert("restore", value(entry.restore));
        repos.push(table);
    }
    Ok(document.to_string())
}

fn set_preserving_comment(table: &mut Table, key: &str, mut replacement: Item) {
    if let Some(previous) = table.get(key).and_then(Item::as_value)
        && let Some(value) = replacement.as_value_mut()
    {
        *value.decor_mut() = previous.decor().clone();
    }
    table.insert(key, replacement);
}
