//! Repository selection and caller-shell directory handoff.

use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use crate::Result;
use crate::discovery::Repository;

pub fn jump(repositories: &[Repository], query: Option<&str>) -> Result<()> {
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
        [] => Err("no repositories match; use `repot list` to see known repositories".into()),
        [repo] => handoff(&repo.path),
        _ if !io::stdin().is_terminal() => Err(format!(
            "{} repositories match; supply an exact name or run `repot jump` in a terminal",
            candidates.len()
        )),
        _ => pick(&candidates, query),
    }
}

fn fuzzy_matches(path: &str, query: &str) -> bool {
    let mut letters = path.chars();
    query
        .chars()
        .all(|letter| letters.any(|item| item == letter))
}

fn pick(candidates: &[&Repository], query: &str) -> Result<()> {
    let mut child = Command::new("fzf")
        .args([
            "--read0",
            "--print0",
            "--no-multi",
            "--tiebreak=index",
            "--prompt=repot> ",
            "--query",
            query,
        ])
        .env_remove("FZF_DEFAULT_OPTS")
        .env_remove("FZF_DEFAULT_OPTS_FILE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| {
            format!("cannot start fzf ({error}); install fzf or supply an exact name")
        })?;
    let input_result = child.stdin.take().map_or_else(
        || Err(io::Error::other("fzf input pipe is unavailable")),
        |mut input| {
            for repo in candidates {
                input.write_all(repo.path.as_os_str().as_encoded_bytes())?;
                input.write_all(&[0])?;
            }
            Ok(())
        },
    );
    let output = child
        .wait_with_output()
        .map_err(|error| format!("cannot wait for fzf: {error}"))?;
    if !output.status.success() {
        return Err("repository selection cancelled or fzf failed".into());
    }
    input_result.map_err(|error| format!("cannot send repository paths to fzf: {error}"))?;
    let selected = output.stdout.strip_suffix(&[0]).unwrap_or(&output.stdout);
    let repo = candidates
        .iter()
        .find(|repo| repo.path.as_os_str().as_encoded_bytes() == selected)
        .ok_or_else(|| "fzf returned an unknown repository path".to_owned())?;
    handoff(&repo.path)
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

const NU_INIT: &str = r"# Run repot and follow its directory handoff after a successful command.
def --env --wrapped repot [...args: string]: nothing -> nothing {
    let repot_cd_file = (^mktemp -t repot-cd.XXXXXXXX | str trim)
    if $env.LAST_EXIT_CODE != 0 { error make {msg: 'Cannot create repot handoff file'} }
    let repot_result = (try {
        with-env {REPOT_CD_FILE: $repot_cd_file} {
            ^repot ...$args | tee { print -n } | tee --stderr { print -e -n } | complete
        }
    } catch {|err| print -e $err.msg; {exit_code: 127} })
    let repot_destination = (try { open --raw $repot_cd_file } catch { '' })
    rm -f $repot_cd_file
    if $repot_result.exit_code == 0 and ($repot_destination | is-not-empty) and ($repot_destination | path type) == 'dir' {
        cd $'($repot_destination)/.'
    }
    $env.LAST_EXIT_CODE = $repot_result.exit_code
}
";
