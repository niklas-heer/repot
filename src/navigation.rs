//! Repository selection and caller-shell directory handoff.

use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::Result;
use crate::discovery::Repository;

mod picker;

pub fn jump(repositories: &[Repository], roots: &[PathBuf], query: Option<&str>) -> Result<()> {
    let path = select(repositories, roots, query)?;
    // Count the jump even without the shell hook; the hook's own record of the
    // same checkout is deduplicated.
    let _ = crate::visits::record(&path);
    handoff(&path)
}

/// Resolve a query to one repository: an exact or unique match directly,
/// otherwise the picker in a terminal, otherwise an error for scripts.
pub fn select(
    repositories: &[Repository],
    roots: &[PathBuf],
    query: Option<&str>,
) -> Result<PathBuf> {
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
        // `owner/name` or `host/owner/name`, as `repot list` prints them, select
        // directly when only one repository has that path.
        let by_path: Vec<_> = repositories
            .iter()
            .filter(|repo| !query.is_empty() && path_matches(&repo.path, roots, query))
            .collect();
        if by_path.len() == 1 {
            by_path
        } else {
            fuzzy_candidates(repositories, &query.to_lowercase())
        }
    } else {
        exact
    };
    match candidates.as_slice() {
        [repo] => Ok(repo.path.clone()),
        _ if io::stdin().is_terminal() && !repositories.is_empty() => {
            picker::pick(repositories, roots, query)
        }
        [] => Err("no repositories match; use `repot list` to see known repositories".into()),
        _ => Err(format!(
            "{} repositories match; supply an exact name or run this in a terminal",
            candidates.len()
        )),
    }
}

/// Whether `query` is the root-relative path of `path` or its trailing whole
/// components. Paths outside every root are compared as they are.
fn path_matches(path: &Path, roots: &[PathBuf], query: &str) -> bool {
    let relative = roots
        .iter()
        .find_map(|root| {
            let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
            path.strip_prefix(&canonical)
                .or_else(|_| path.strip_prefix(root))
                .ok()
        })
        .unwrap_or(path);
    relative.ends_with(query)
}

/// Repositories whose file name or, failing that, whole path fuzzy-matches the
/// lowercased query.
fn fuzzy_candidates<'a>(repositories: &'a [Repository], query: &str) -> Vec<&'a Repository> {
    let names: Vec<_> = repositories
        .iter()
        .filter(|repo| {
            repo.path
                .file_name()
                .is_some_and(|name| fuzzy_matches(&name.to_string_lossy().to_lowercase(), query))
        })
        .collect();
    if names.is_empty() {
        repositories
            .iter()
            .filter(|repo| fuzzy_matches(&repo.path.to_string_lossy().to_lowercase(), query))
            .collect()
    } else {
        names
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

pub fn shell_init(shell: &str, track: bool) -> Result<()> {
    let (script, hook) = match shell {
        "bash" => (POSIX_INIT.to_owned(), BASH_TRACK),
        "zsh" => (POSIX_INIT.to_owned(), ZSH_TRACK),
        "fish" => (FISH_INIT.to_owned(), FISH_TRACK),
        "nu" => (nu_init(), NU_TRACK),
        _ => {
            return Err(format!(
                "unsupported shell {shell:?}; choose nu, zsh, bash, or fish"
            ));
        }
    };
    let mut output = io::stdout().lock();
    output
        .write_all(script.as_bytes())
        .and_then(|()| {
            if track {
                output.write_all(hook.as_bytes())
            } else {
                Ok(())
            }
        })
        .map_err(|error| format!("cannot print shell integration: {error}"))
}

// Directory-change hooks feed the picker's frecency ranking. They call the
// binary directly, never the wrapper, and stay silent on any failure.
// The bash hook must return the status it received: it runs ahead of the
// user's own PROMPT_COMMAND, which may read `$?`. Bash 5.1+ allows an array.
const BASH_TRACK: &str = r#"
# Remember which repositories you use, to rank `repot cd` (disable: --no-track).
__repot_visit() {
    local repot_status=$?
    if [ "${__repot_last_pwd-}" != "$PWD" ]; then
        __repot_last_pwd=$PWD
        command repot visit -- "$PWD" >/dev/null 2>&1
    fi
    return "$repot_status"
}
case "$(declare -p PROMPT_COMMAND 2>/dev/null)" in
    "declare -a"*)
        __repot_listed=
        for __repot_entry in ${PROMPT_COMMAND[@]+"${PROMPT_COMMAND[@]}"}; do
            if [ "$__repot_entry" = __repot_visit ]; then __repot_listed=1; fi
        done
        if [ -z "$__repot_listed" ]; then
            PROMPT_COMMAND=(__repot_visit ${PROMPT_COMMAND[@]+"${PROMPT_COMMAND[@]}"})
        fi
        unset __repot_listed __repot_entry
        ;;
    *)
        case ";${PROMPT_COMMAND-};" in
            *";__repot_visit;"*) ;;
            *) PROMPT_COMMAND="__repot_visit${PROMPT_COMMAND:+;$PROMPT_COMMAND}" ;;
        esac
        ;;
esac
"#;

// The hooks also record the directory the shell starts in, which no change event reports.
const ZSH_TRACK: &str = r#"
# Remember which repositories you use, to rank `repot cd` (disable: --no-track).
__repot_visit() { command repot visit -- "$PWD" >/dev/null 2>&1 }
autoload -Uz add-zsh-hook
add-zsh-hook chpwd __repot_visit
__repot_visit
"#;

const FISH_TRACK: &str = r"
# Remember which repositories you use, to rank `repot cd` (disable: --no-track).
function __repot_visit --on-variable PWD
    command repot visit -- $PWD >/dev/null 2>&1
end
__repot_visit
";

// Re-sourcing replaces the earlier hook. Only closures carrying the marker
// comment are repot's; records and other closures belong to the user.
const NU_TRACK: &str = r"
# Remember which repositories you use, to rank `repot cd` (disable: --no-track).
$env.config.hooks.env_change.PWD = (
    $env.config.hooks.env_change.PWD?
    | default []
    | where {|hook| ($hook | describe) != 'closure' or not ((view source $hook) | str contains '__repot_visit_hook') }
    | append {|_, after|
        # __repot_visit_hook: replaced when this integration is sourced again
        try { ^repot visit -- $after o+e>| ignore }
    }
)
";

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
        cd -- "$repot_destination"
    end
    return $repot_status
end
"#;

/// Subcommands that can hand off a directory. Nushell gets a `def --env`
/// wrapper for each; every other `repot` call runs the binary directly, so its
/// output streams and pipes like any external command and the generated
/// `extern "repot"` completions keep working.
pub const NU_HANDOFF: &[&str] = &[
    "cd",
    "jump",
    "clone",
    "get",
    "new",
    "create",
    "publish",
    "adopt",
    "migrate",
    "trash restore",
];

// Wrapped commands stay attached to the terminal, so the picker, colour and
// live output work. With `--json` they return the report as a value instead.
// A failure raises an error, which `try` can catch.
const NU_INIT: &str = r#"# repot shell integration: lets `repot cd`, `new`, `clone` and friends change directory.
# Report commands exit 3 when something needs review, which nu raises as an error;
# read the status with `repot status --json | complete` instead of `| from json`.
# Put global options after the subcommand, e.g. `repot cd --manifest PATH query`.
def --env __repot_handoff [args: list<string>] {
    let cd_file = (mktemp -t repot-cd.XXXXXXXX)
    let run = if '--json' in $args {
        try {
            {status: 0, output: (with-env {REPOT_CD_FILE: $cd_file} { ^repot ...$args })}
        } catch {|error| {status: ($error.exit_code? | default 1), output: null} }
    } else {
        try {
            with-env {REPOT_CD_FILE: $cd_file} { ^repot ...$args }
            {status: 0, output: null}
        } catch {|error| {status: ($error.exit_code? | default 1), output: null} }
    }
    let destination = (try { open --raw $cd_file } catch { '' })
    rm -f $cd_file
    if $run.status != 0 {
        error make --unspanned {msg: $"repot ($args | first) exited with status ($run.status)"}
    }
    # Follow symlinked checkouts; the `/.` keeps names ending in a newline intact.
    if ($destination | is-not-empty) and ($destination | path expand | path type) == 'dir' {
        cd $'($destination)/.'
    }
    $run.output
}
"#;

/// Repository names for Tab completion, shared by shell-init and the generated
/// completions module. Failures yield no names, so Tab stays quiet.
pub const NU_REPO_NAMES: &str =
    "def __repot_repos [] { try { ^repot list --unique err> /dev/null | lines } catch { [] } }";

fn nu_init() -> String {
    let mut script = String::from(NU_INIT);
    script.push_str(NU_REPO_NAMES);
    script.push('\n');
    for name in NU_HANDOFF {
        let words = name
            .split(' ')
            .map(|word| format!("'{word}'"))
            .collect::<Vec<_>>();
        // `cd` and `jump` take a repository query; the other handoffs take paths or new names.
        let completer = if matches!(*name, "cd" | "jump") {
            "@__repot_repos"
        } else {
            ""
        };
        let wrapper = format!(
            "def --env --wrapped \"repot {name}\" [...args: string{completer}] {{ __repot_handoff [{} ...$args] }}\n",
            words.join(" ")
        );
        script.push_str(&wrapper);
    }
    script
}
