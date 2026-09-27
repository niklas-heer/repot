//! Repository selection and caller-shell directory handoff.

use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::Result;
use crate::discovery::Repository;

mod picker;

pub fn jump(repositories: &[Repository], roots: &[PathBuf], query: Option<&str>) -> Result<()> {
    let query = query.unwrap_or_default();
    let exact: Vec<_> = repositories
        .iter()
        .filter(|repo| {
            !query.is_empty()
                && (repo.path == Path::new(query)
                    || repo.path.file_name().is_some_and(|name| name == query))
        })
        .collect();
    let candidates: Vec<_> = if exact.is_empty() {
        let query = query.to_lowercase();
        let names: Vec<_> = repositories
            .iter()
            .filter(|repo| {
                repo.path.file_name().is_some_and(|name| {
                    fuzzy_matches(&name.to_string_lossy().to_lowercase(), &query)
                })
            })
            .collect();
        if names.is_empty() {
            repositories
                .iter()
                .filter(|repo| fuzzy_matches(&repo.path.to_string_lossy().to_lowercase(), &query))
                .collect()
        } else {
            names
        }
    } else {
        exact
    };
    match candidates.as_slice() {
        [repo] => handoff(&repo.path),
        _ if io::stdin().is_terminal() && !repositories.is_empty() => {
            let selected = picker::pick(repositories, roots, query)?;
            handoff(&selected)
        }
        [] => Err("no repositories match; use `repot list` to see known repositories".into()),
        _ => Err(format!(
            "{} repositories match; supply an exact name or run `repot jump` in a terminal",
            candidates.len()
        )),
    }
}

fn fuzzy_matches(path: &str, query: &str) -> bool {
    let mut letters = path.chars();
    query
        .chars()
        .all(|letter| letters.any(|item| item == letter))
}

pub fn handoff(path: &Path) -> Result<()> {
    if !path.is_dir() {
        return Err(format!("directory no longer exists: {}", path.display()));
    }
    let bytes = path.as_os_str().as_encoded_bytes();
    env::var_os("REPOT_CD_FILE").map_or_else(
        || {
            let mut output = io::stdout().lock();
            output
                .write_all(bytes)
                .and_then(|()| output.write_all(b"\n"))
                .map_err(|error| format!("cannot print repository path: {error}"))
        },
        |file| {
            fs::write(file, bytes)
                .map_err(|error| format!("cannot write shell directory handoff: {error}"))
        },
    )
}

pub fn shell_init(shell: &str) -> Result<()> {
    let script = match shell {
        "bash" | "zsh" => POSIX_INIT,
        "fish" => FISH_INIT,
        "nu" => NU_INIT,
        _ => {
            return Err(format!(
                "unsupported shell {shell:?}; choose nu, zsh, bash, or fish"
            ));
        }
    };
    io::stdout()
        .write_all(script.as_bytes())
        .map_err(|error| format!("cannot print shell integration: {error}"))
}

const POSIX_INIT: &str = r#"repot() {
    local repot_cd_file repot_status repot_destination
    repot_cd_file=$(mktemp "${TMPDIR:-/tmp}/repot-cd.XXXXXXXX") || return
    if REPOT_CD_FILE="$repot_cd_file" command repot "$@"; then
        repot_status=0
    else
        repot_status=$?
    fi
    repot_destination=$(cat "$repot_cd_file"; printf '.')
    repot_destination=${repot_destination%.}
    command rm -f -- "$repot_cd_file"
    if [ "$repot_status" -eq 0 ] && [ -n "$repot_destination" ] && [ -d "$repot_destination" ]; then
        builtin cd -- "$repot_destination"
    fi
    return "$repot_status"
}
"#;

const FISH_INIT: &str = r#"function repot
    set -l repot_tmp_dir /tmp
    if set -q TMPDIR; and test -n "$TMPDIR"
        set repot_tmp_dir "$TMPDIR"
    end
    set -l repot_cd_file (mktemp "$repot_tmp_dir/repot-cd.XXXXXXXX")
    or return $status
    command env REPOT_CD_FILE="$repot_cd_file" repot $argv
    set -l repot_status $status
    set -l repot_destination (string collect --allow-empty --no-trim-newlines < "$repot_cd_file")
    command rm -f -- "$repot_cd_file"
    if test "$repot_status" -eq 0; and test -n "$repot_destination"; and test -d "$repot_destination"
        builtin cd -- "$repot_destination"
    end
    return $repot_status
end
"#;

const NU_INIT: &str = r"# repot shell integration: lets `repot cd`, `new`, `clone` and friends change directory.
def --env --wrapped repot [...args: string] {
    let command = ($args | where {|arg| not ($arg | str starts-with '-') } | get 0? | default '')
    if $command in [list ls root status sync scan find restore completions shell-init mcp agent-guide help h] {
        # Reports stay attached to the terminal and remain pipeable, e.g. `repot status --json | from json`.
        ^repot ...$args
    } else {
        let repot_cd_file = (^mktemp -t repot-cd.XXXXXXXX | str trim)
        try { with-env {REPOT_CD_FILE: $repot_cd_file} { ^repot ...$args } } catch { }
        let repot_status = $env.LAST_EXIT_CODE
        let repot_destination = (try { open --raw $repot_cd_file } catch { '' })
        rm -f $repot_cd_file
        if $repot_status == 0 and ($repot_destination | is-not-empty) and ($repot_destination | path type) == 'dir' {
            cd $'($repot_destination)/.'
        }
        $env.LAST_EXIT_CODE = $repot_status
    }
}
";
