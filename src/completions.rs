//! Shell completion and help rendering from the authoritative CLI definition.

use std::io::{self, Write};

use clap::{Command, ValueEnum};
use clap_complete::Generator;

use crate::Result;
use crate::navigation::{NU_HANDOFF, NU_REPO_NAMES};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    Nu,
}

/// Subcommands whose positional QUERY names one repository; Tab completes those
/// names. `jump` is the visible alias of `cd`.
const REPO_QUERY_COMMANDS: &[&str] = &["cd", "info", "open", "rm"];

/// Names from `repot list --unique`, which `repot cd` accepts. Failures print
/// nothing so a Tab press never shows errors.
const REPO_NAMES_COMMAND: &str = "command repot list --unique 2>/dev/null";

const BASH_REPO_NAMES: &str = r#"
# Complete repository names for the query of cd (jump), info, open and rm; every
# other word completes as the generated function does.
_repot_repos() {
    local i word cur plain line escaped subcommand=
    cur="${COMP_WORDS[COMP_CWORD]}"
    for ((i = 1; i < COMP_CWORD; i++)); do
        word="${COMP_WORDS[i]}"
        case "$word" in
            --manifest) i=$((i + 1)) ;;
            -*) ;;
            *) subcommand="$word"; break ;;
        esac
    done
    case "$subcommand" in
        cd|jump|info|open|rm) ;;
        *) _repot "$@"; return ;;
    esac
    case "$cur" in
        -*) _repot "$@"; return ;;
    esac
    # Readline quotes the word itself, so match on the text the user typed.
    plain="${cur//\\/}"
    plain="${plain#\"}"
    plain="${plain#\'}"
    COMPREPLY=()
    while IFS= read -r line; do
        if [[ "$line" == "$plain"* ]]; then
            printf -v escaped '%q' "$line"
            COMPREPLY+=("$escaped")
        fi
    done < <(%REPO_NAMES_COMMAND%)
}
"#;

const ZSH_REPO_NAMES: &str = r#"_repot_repos() {
    compadd -- ${(f)"$(%REPO_NAMES_COMMAND%)"}
}
"#;

pub fn render(shell: Shell, mut command: Command) -> Result<()> {
    command.set_bin_name("repot");
    command.build();
    let text = match shell {
        Shell::Bash => wire_bash(&generate(&clap_complete::shells::Bash, &command)?),
        Shell::Zsh => wire_zsh(&generate(&clap_complete::shells::Zsh, &command)?),
        Shell::Fish => wire_fish(&generate(&clap_complete::shells::Fish, &command)?),
        Shell::Nu => wire_nu(&render_nu(&command)?),
    };
    io::stdout()
        .lock()
        .write_all(text.as_bytes())
        .map_err(|error| format!("cannot print completions: {error}"))
}

fn generate(generator: &dyn Generator, command: &Command) -> Result<String> {
    let mut buffer = Vec::new();
    generator
        .try_generate(command, &mut buffer)
        .map_err(|error| format!("cannot print completions: {error}"))?;
    String::from_utf8(buffer).map_err(|error| format!("cannot print completions: {error}"))
}

fn repo_names_zsh() -> String {
    ZSH_REPO_NAMES.replace("%REPO_NAMES_COMMAND%", REPO_NAMES_COMMAND)
}

fn repo_names_bash() -> String {
    BASH_REPO_NAMES.replace("%REPO_NAMES_COMMAND%", REPO_NAMES_COMMAND)
}

/// Re-registers the generated `_repot` under a wrapper that completes repository
/// names and delegates everything else. Output the wrapper cannot place is kept
/// unwired, so the script still loads.
fn wire_bash(generated: &str) -> String {
    let registrations = generated.matches("complete -F _repot ").count();
    if registrations != 2 {
        return generated.to_owned();
    }
    let mut wired = generated.replace("complete -F _repot ", "complete -F _repot_repos ");
    wired.push_str(&repo_names_bash());
    wired
}

/// Adds the repository-name function to every query command's `_arguments` spec.
/// Each query command must be wired exactly once, or the output stays unwired.
fn wire_zsh(generated: &str) -> String {
    wire_zsh_specs(generated).unwrap_or_else(|| generated.to_owned())
}

fn wire_zsh_specs(generated: &str) -> Option<String> {
    let mut output = String::new();
    let mut block: Option<&str> = None;
    let mut wired = Vec::new();
    let mut functions_inserted = false;
    for line in generated.split_inclusive('\n') {
        let trimmed = line.trim();
        if let Some(label) = trimmed
            .strip_prefix('(')
            .and_then(|rest| rest.strip_suffix(')'))
        {
            block = REPO_QUERY_COMMANDS
                .iter()
                .copied()
                .find(|name| *name == label);
            if label == "cd" {
                // `cd` and its alias share one `case` branch.
                output.push_str(&line.replacen("(cd)", "(cd|jump)", 1));
                continue;
            }
        } else if trimmed == "_repot() {" {
            output.push_str(&repo_names_zsh());
            output.push('\n');
            functions_inserted = true;
        }
        if let Some(name) = block
            && trimmed.contains("query -- ")
            && let Some(prefix) = line.strip_suffix(":_default' \\\n")
        {
            output.push_str(prefix);
            output.push_str(":_repot_repos' \\\n");
            wired.push(name);
            continue;
        }
        output.push_str(line);
    }
    let complete = functions_inserted
        && wired.len() == REPO_QUERY_COMMANDS.len()
        && REPO_QUERY_COMMANDS
            .iter()
            .all(|name| wired.iter().filter(|wired| *wired == name).count() == 1);
    complete.then_some(output)
}

/// Fish completes names for every positional word after the query commands;
/// `-f` keeps file names out of those positions.
fn wire_fish(generated: &str) -> String {
    let names = REPO_QUERY_COMMANDS
        .iter()
        .chain(&["jump"])
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "{generated}complete -c repot -n '__fish_repot_using_subcommand {names}' -f -a '({REPO_NAMES_COMMAND})'\n"
    )
}

/// The Nushell module. Nushell resolves `extern "repot cd"` ahead of the
/// shell-init wrapper of the same name, so the generated definitions for
/// subcommands that hand off a directory are left out; the wrapper then handles
/// them whichever file is sourced first. `extern "repot"` stays, so other
/// subcommands still complete.
fn render_nu(command: &Command) -> Result<String> {
    let generated = generate(&clap_complete_nushell::Nushell, command)?;
    let mut output = String::new();
    let mut skipping = false;
    for line in generated.split_inclusive('\n') {
        if skipping {
            skipping = line != "  ]\n";
            continue;
        }
        let wrapped = line
            .strip_prefix("  export extern ")
            .and_then(|rest| rest.strip_suffix(" [\n"))
            .map(|name| name.trim_matches('"'))
            .and_then(|name| name.strip_prefix("repot "))
            .is_some_and(|name| NU_HANDOFF.contains(&name));
        if wrapped {
            skipping = true;
            continue;
        }
        output.push_str(line);
    }
    Ok(output)
}

/// Gives each query extern a repository-name completer. The helper lives inside
/// the module, so the module works without the shell-init file.
fn wire_nu(generated: &str) -> String {
    wire_nu_queries(generated).unwrap_or_else(|| generated.to_owned())
}

fn wire_nu_queries(generated: &str) -> Option<String> {
    const HEADER: &str = "module completions {\n";
    if generated.matches(HEADER).count() != 1 {
        return None;
    }
    let expected: Vec<_> = REPO_QUERY_COMMANDS
        .iter()
        .filter(|name| !NU_HANDOFF.contains(name))
        .copied()
        .collect();
    let mut output = String::new();
    let mut extern_name: Option<&str> = None;
    let mut wired = Vec::new();
    for line in generated.split_inclusive('\n') {
        if line == HEADER {
            output.push_str(line);
            output.push_str("  ");
            output.push_str(NU_REPO_NAMES);
            output.push('\n');
            continue;
        }
        if let Some(name) = line
            .strip_prefix("  export extern \"repot ")
            .and_then(|rest| rest.strip_suffix("\" [\n"))
        {
            extern_name = expected.iter().copied().find(|query| *query == name);
        } else if line == "  ]\n" {
            extern_name = None;
        } else if let Some(name) = extern_name
            && line.starts_with("    query")
            && line.contains(": string")
        {
            output.push_str(&line.replacen(": string", ": string@__repot_repos", 1));
            wired.push(name);
            extern_name = None;
            continue;
        }
        output.push_str(line);
    }
    (wired.len() == expected.len()
        && expected
            .iter()
            .all(|name| wired.iter().filter(|wired| *wired == name).count() == 1))
    .then_some(output)
}

pub fn help(path: &[String], mut command: Command) -> Result<()> {
    command.build();
    let mut current = &mut command;
    for name in path {
        current = current
            .find_subcommand_mut(name)
            .ok_or_else(|| format!("unknown command {name:?}"))?;
    }
    let help = current.render_long_help();
    let text = if crate::ui::Paint::stdout().colour() {
        help.ansi().to_string()
    } else {
        help.to_string()
    };
    writeln!(io::stdout().lock(), "{}", text.trim_end())
        .map_err(|error| format!("cannot print help: {error}"))
}
