//! Open a repository in your editor or on its forge's web page.

use std::env;
use std::ffi::OsString;
use std::fmt::Write as _;
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

/// Splits a command line into words with POSIX-shell quoting: whitespace
/// separates words, single quotes are literal, double quotes accept `\"`, `\\`,
/// `\$`, `` \` `` and `\<newline>` escapes, and a backslash escapes the next
/// character. Nothing is expanded and no shell runs. `None` if a quote is left
/// open or the text ends in a backslash.
fn split_words(text: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        match character {
            ' ' | '\t' | '\n' => words.extend(word.take()),
            '\'' => {
                let current = word.get_or_insert_with(String::new);
                loop {
                    match characters.next()? {
                        '\'' => break,
                        literal => current.push(literal),
                    }
                }
            }
            '"' => {
                let current = word.get_or_insert_with(String::new);
                loop {
                    match characters.next()? {
                        '"' => break,
                        '\\' => match characters.next()? {
                            escaped @ ('"' | '\\' | '$' | '`') => current.push(escaped),
                            '\n' => {}
                            other => {
                                current.push('\\');
                                current.push(other);
                            }
                        },
                        literal => current.push(literal),
                    }
                }
            }
            '\\' => match characters.next()? {
                '\n' => {}
                escaped => word.get_or_insert_with(String::new).push(escaped),
            },
            literal => word.get_or_insert_with(String::new).push(literal),
        }
    }
    words.extend(word);
    Some(words)
}

/// Parses an environment variable holding a command line into argv.
fn command_words(name: &str, text: &str) -> Result<Vec<OsString>> {
    let words = split_words(text)
        .ok_or_else(|| format!("{name} has an unterminated quote or trailing backslash"))?;
    Ok(words.into_iter().map(OsString::from).collect())
}

/// `REPOT_EDITOR`, then `VISUAL`, then `EDITOR`, split into program and flags.
fn editor() -> Result<Vec<OsString>> {
    let (name, value) = ["REPOT_EDITOR", "VISUAL", "EDITOR"]
        .into_iter()
        .find_map(|name| {
            env::var_os(name)
                .filter(|value| !value.is_empty())
                .map(|value| (name, value))
        })
        .ok_or("no editor configured; set REPOT_EDITOR, VISUAL or EDITOR, or use --web")?;
    let words = command_words(name, &value.to_string_lossy())?;
    if words.is_empty() {
        return Err(
            "no editor configured; set REPOT_EDITOR, VISUAL or EDITOR, or use --web".into(),
        );
    }
    Ok(words)
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
        page.push_str(&encode_branch(&branch));
    }
    Ok(page)
}

/// Percent-encodes each segment of a branch name and keeps the `/` separators,
/// so characters such as `#` and `%` reach the forge as part of the path.
fn encode_branch(branch: &str) -> String {
    branch
        .split('/')
        .map(|segment| {
            let mut encoded = String::new();
            for byte in segment.bytes() {
                if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                    encoded.push(char::from(byte));
                } else {
                    let _ = write!(encoded, "%{byte:02X}");
                }
            }
            encoded
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn browser() -> Result<Vec<OsString>> {
    if let Some(browser) = env::var_os("BROWSER").filter(|value| !value.is_empty()) {
        return command_words("BROWSER", &browser.to_string_lossy());
    }
    Ok(vec![OsString::from(if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    })])
}

fn launch(words: &[OsString], argument: &OsString) -> Result<()> {
    let (program, flags) = words.split_first().ok_or("no program to launch")?;
    let status = Command::new(program)
        .args(flags)
        .arg(argument)
        // The editor or browser may run `repot cd`, which must not move the caller's shell.
        .env_remove("REPOT_CD_FILE")
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
        launch(&browser()?, &OsString::from(page))?;
    } else {
        let editor = editor()?;
        ui::note("↗", ui::INFO, &format!("opening {name}"));
        launch(&editor, &path.as_os_str().to_owned())?;
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Option<Vec<String>> {
        split_words(text)
    }

    #[test]
    fn splits_plain_words_and_ignores_extra_whitespace() {
        assert_eq!(
            words("  code\t--wait\n"),
            Some(vec!["code".into(), "--wait".into()])
        );
        assert_eq!(words("   "), Some(vec![]));
        assert_eq!(words(""), Some(vec![]));
    }

    #[test]
    fn single_quotes_are_literal() {
        assert_eq!(
            words(r"'/a b/ed\it' --x"),
            Some(vec![r"/a b/ed\it".into(), "--x".into()])
        );
        assert_eq!(words("''"), Some(vec![String::new()]));
    }

    #[test]
    fn double_quotes_keep_only_the_shell_escapes() {
        assert_eq!(
            words(r#""/a b/say \"hi\"" "x\y\$z\`""#),
            Some(vec![r#"/a b/say "hi""#.into(), r"x\y$z`".into(),])
        );
        assert_eq!(words(r#""a\\b""#), Some(vec![r"a\b".into()]));
    }

    #[test]
    fn backslash_escapes_outside_quotes_and_joins_words() {
        assert_eq!(words(r"/a\ b/ed"), Some(vec!["/a b/ed".into()]));
        assert_eq!(
            words(r#"pre'fix'"mid"post"#),
            Some(vec!["prefixmidpost".into()])
        );
        assert_eq!(words("a\\\nb"), Some(vec!["ab".into()]));
    }

    #[test]
    fn unterminated_quotes_and_trailing_backslashes_are_rejected() {
        assert_eq!(words("'open"), None);
        assert_eq!(words("\"open"), None);
        assert_eq!(words("\"open\\"), None);
        assert_eq!(words("code\\"), None);
    }

    #[test]
    fn encodes_each_branch_segment() {
        assert_eq!(
            encode_branch("feature/plain-1.0_~x"),
            "feature/plain-1.0_~x"
        );
        assert_eq!(encode_branch("feat#1%25/x"), "feat%231%2525/x");
        assert_eq!(encode_branch("a b/c+d"), "a%20b/c%2Bd");
        assert_eq!(encode_branch("ünï/x"), "%C3%BCn%C3%AF/x");
    }
}
