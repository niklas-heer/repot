//! Generated completions load in real shells without reading repository config.

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::process::{Command, Output};

    fn run(home: &std::path::Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_repot"))
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("XDG_STATE_HOME", home.join("state"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", home.join("gitconfig"))
            .args(args)
            .output()
            .expect("repot")
    }

    #[test]
    fn completion_scripts_load_in_each_available_shell_with_invalid_manifest() {
        let home = tempfile::tempdir().expect("temporary home");
        let manifest = home.path().join("broken.toml");
        fs::write(&manifest, "broken = [").expect("invalid manifest");
        for shell in ["bash", "zsh", "fish", "nu"] {
            let output = run(
                home.path(),
                &[
                    "--manifest",
                    manifest.to_str().expect("path"),
                    "completions",
                    shell,
                ],
            );
            assert!(
                output.status.success(),
                "{shell}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let text = String::from_utf8(output.stdout).expect("completion UTF-8");
            for command in [
                "clone",
                "new",
                "cd",
                "scan",
                "trash",
                "publish",
                "completions",
                "agent-guide",
            ] {
                assert!(text.contains(command), "{shell} includes {command}");
            }
            let path = home.path().join(format!("repot.{shell}"));
            fs::write(&path, text).expect("completion file");
            if Command::new(shell).arg("--version").output().is_err() {
                assert!(
                    env::var("REPOT_REQUIRE_SHELLS").as_deref() != Ok("1"),
                    "{shell} is required because REPOT_REQUIRE_SHELLS=1"
                );
                eprintln!("skipping optional {shell}: unavailable");
                continue;
            }
            let mut command = Command::new(shell);
            command
                .env("HOME", home.path())
                .env("XDG_CONFIG_HOME", home.path().join("config"))
                .env("XDG_STATE_HOME", home.path().join("state"));
            match shell {
                "bash" => {
                    command
                        .args([
                            "--noprofile",
                            "--norc",
                            "-c",
                            "source \"$1\"; complete -p repot",
                            "completion-test",
                        ])
                        .arg(&path);
                }
                "zsh" => {
                    command
                        .args([
                            "-f",
                            "-c",
                            "autoload -Uz compinit; compinit -D; source \"$1\"; whence -w _repot",
                            "completion-test",
                        ])
                        .arg(&path);
                }
                "fish" => {
                    command
                        .args(["--no-config", "-c", "source $argv[1]; complete -C 'repot '"])
                        .arg(&path);
                }
                "nu" => {
                    command.arg("--no-config-file").arg(&path);
                }
                _ => {}
            }
            let loaded = command.output().expect("load completions");
            assert!(
                loaded.status.success(),
                "{shell}: {}",
                String::from_utf8_lossy(&loaded.stderr)
            );
        }
    }

    #[test]
    fn nushell_completions_leave_directory_changing_commands_to_the_wrapper() {
        let home = tempfile::tempdir().expect("temporary home");
        let output = run(home.path(), &["completions", "nu"]);
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).expect("completion UTF-8");
        // An `extern` with the same name would run the binary directly and bypass
        // the shell-init wrapper, whichever file is sourced last.
        for name in [
            "repot cd",
            "repot jump",
            "repot clone",
            "repot get",
            "repot new",
            "repot publish",
            "repot adopt",
            "repot create",
            "repot migrate",
            "repot trash restore",
        ] {
            assert!(
                !text.contains(&format!("export extern \"{name}\" [")),
                "{name} is left to the wrapper"
            );
        }
        // The top-level extern stays, so `repot <Tab>` still lists subcommands.
        assert!(text.contains("export extern repot ["));
        for name in ["repot list", "repot status", "repot trash list"] {
            assert!(
                text.contains(&format!("export extern \"{name}\" [")),
                "{name}"
            );
        }
    }

    #[test]
    fn version_and_help_aliases_work_without_loading_manifest() {
        let home = tempfile::tempdir().expect("temporary home");
        let manifest = home.path().join("broken.toml");
        fs::write(&manifest, "broken = [").expect("invalid manifest");
        let guide = run(
            home.path(),
            &[
                "--manifest",
                manifest.to_str().expect("path"),
                "agent-guide",
            ],
        );
        assert!(guide.status.success());
        let guide = String::from_utf8(guide.stdout).expect("agent guide UTF-8");
        assert!(guide.contains("--dry-run"));
        assert!(guide.contains("--json"));
        for flag in ["-v", "-V", "--version"] {
            let output = run(home.path(), &["--manifest", "/missing/manifest", flag]);
            assert!(output.status.success(), "{flag}");
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).trim(),
                concat!("repot ", env!("CARGO_PKG_VERSION"))
            );
        }
        for args in [
            vec!["help", "get"],
            vec!["h", "get"],
            vec!["help", "trash", "restore"],
        ] {
            let output = run(home.path(), &args);
            assert!(
                output.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains("--dry-run"));
        }
        assert!(
            !run(home.path(), &["completions", "unknown-shell"])
                .status
                .success()
        );
        assert!(
            !run(home.path(), &["help", "unknown-command"])
                .status
                .success()
        );
    }

    /// Repositories that the query completions and their selections use.
    const REPOSITORIES: &[&str] = &[
        "github.com/owner/alpha",
        "github.com/owner/alphabet",
        "github.com/owner/beta",
        "github.com/other/my repo",
    ];

    fn repositories(home: &std::path::Path) -> std::path::PathBuf {
        let root = home.join("projects");
        for name in REPOSITORIES {
            let path = root.join(name);
            fs::create_dir_all(&path).expect("checkout directory");
            let output = Command::new("git")
                .arg("init")
                .arg("--quiet")
                .arg(&path)
                .output()
                .expect("git init runs");
            assert!(output.status.success());
        }
        root
    }

    /// A shell command with the repot binary first on PATH and an isolated home.
    fn shell_command(program: &str, home: &std::path::Path) -> Command {
        let binary_dir = std::path::Path::new(env!("CARGO_BIN_EXE_repot"))
            .parent()
            .expect("binary directory")
            .to_path_buf();
        let path = env::join_paths(
            std::iter::once(binary_dir)
                .chain(env::split_paths(&env::var_os("PATH").unwrap_or_default())),
        )
        .expect("test PATH");
        let mut command = Command::new(program);
        command
            .env("PATH", path)
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("XDG_STATE_HOME", home.join("state"))
            .env("GHQ_ROOT", home.join("projects"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", home.join("gitconfig"))
            .env_remove("REPOT_CD_FILE");
        command
    }

    fn write_completions(home: &std::path::Path, shell: &str) -> std::path::PathBuf {
        let output = run(home, &["completions", shell]);
        assert!(output.status.success(), "{shell} completions");
        let path = home.join(format!("repot.{shell}"));
        fs::write(&path, output.stdout).expect("completion file");
        path
    }

    /// Calls the function bash registered for `repot` with the given words, the
    /// last one being the word under the cursor, as bash does for a Tab press.
    fn bash_completion(
        program: &str,
        home: &std::path::Path,
        completion: &std::path::Path,
        words: &[&str],
    ) -> Output {
        let quoted = words
            .iter()
            .map(|word| format!("'{word}'"))
            .collect::<Vec<_>>()
            .join(" ");
        let cursor = words.len().saturating_sub(1);
        let script = format!(
            r#"set -u
source "$1"
COMP_WORDS=({quoted})
COMP_CWORD={cursor}
COMP_LINE="${{COMP_WORDS[*]}}"
COMP_POINT=${{#COMP_LINE}}
COMPREPLY=()
spec=$(complete -p repot)
set -- $spec
while [ "$1" != -F ]; do shift; done
"$2" repot "${{COMP_WORDS[COMP_CWORD]}}" "${{COMP_WORDS[COMP_CWORD-1]}}"
if [ ${{#COMPREPLY[@]}} -gt 0 ]; then printf '%s\n' "${{COMPREPLY[@]}}"; fi
"#
        );
        let script_path = home.join("bash-completion-test");
        fs::write(&script_path, script).expect("bash test script");
        shell_command(program, home)
            .arg(&script_path)
            .arg(completion)
            .output()
            .expect("bash runs")
    }

    fn bash_replies(
        program: &str,
        home: &std::path::Path,
        completion: &std::path::Path,
        words: &[&str],
    ) -> Vec<String> {
        let output = bash_completion(program, home, completion, words);
        assert!(
            output.status.success(),
            "bash completion: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("completion UTF-8")
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// `bash` from PATH, plus the system bash on macOS, which is 3.2.
    fn bash_programs() -> Vec<&'static str> {
        let mut programs = vec!["bash"];
        if std::path::Path::new("/bin/bash").exists() {
            programs.push("/bin/bash");
        }
        programs
    }

    #[test]
    fn bash_completes_repository_names_for_query_commands_and_delegates_the_rest() {
        let home = tempfile::tempdir().expect("temporary home");
        repositories(home.path());
        let completion = write_completions(home.path(), "bash");
        for program in bash_programs() {
            let replies = |words: &[&str]| bash_replies(program, home.path(), &completion, words);
            assert_eq!(
                replies(&["repot", "cd", "al"]),
                ["alpha", "alphabet"],
                "{program}"
            );
            assert_eq!(replies(&["repot", "jump", "be"]), ["beta"], "{program}");
            // Names with spaces come back escaped for the command line.
            assert_eq!(replies(&["repot", "rm", "my"]), ["my\\ repo"], "{program}");
            assert_eq!(replies(&["repot", "info", ""]).len(), REPOSITORIES.len());
            // Flags and other subcommands still come from the generated function.
            let flags = replies(&["repot", "status", "--j"]);
            assert!(flags.contains(&"--json".to_owned()), "{program}: {flags:?}");
            assert!(!flags.iter().any(|reply| reply.contains("alpha")));
            assert!(replies(&["repot", "cd", "--j"]).is_empty(), "{program}");
        }
    }

    #[test]
    fn bash_completion_skips_the_global_manifest_option_and_stays_quiet_on_errors() {
        let home = tempfile::tempdir().expect("temporary home");
        repositories(home.path());
        let completion = write_completions(home.path(), "bash");
        let empty = home.path().join("empty.toml");
        fs::write(&empty, "").expect("empty manifest");
        let empty = empty.to_str().expect("path");
        // `--manifest VALUE` before the subcommand must not hide the subcommand.
        for program in bash_programs() {
            assert_eq!(
                bash_replies(
                    program,
                    home.path(),
                    &completion,
                    &["repot", "--manifest", empty, "cd", "al"]
                ),
                ["alpha", "alphabet"],
                "{program}"
            );
        }
        // With a broken default manifest, a Tab press prints no names and no errors.
        let config = home.path().join("config/repot");
        fs::create_dir_all(&config).expect("config directory");
        fs::write(config.join("repos.toml"), "broken = [").expect("broken manifest");
        for program in bash_programs() {
            let output = bash_completion(program, home.path(), &completion, &["repot", "cd", "al"]);
            assert!(output.status.success(), "{program}");
            assert!(output.stdout.is_empty(), "{program}");
            assert!(
                output.stderr.is_empty(),
                "{program}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn zsh_completion_wires_repository_names_into_every_query_command() {
        let home = tempfile::tempdir().expect("temporary home");
        let output = run(home.path(), &["completions", "zsh"]);
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).expect("completion UTF-8");
        assert!(text.contains(
            "_repot_repos() {\n    compadd -- ${(f)\"$(command repot list --unique 2>/dev/null)\"}\n}\n"
        ));
        // Each query command's `_arguments` spec names the function; `list` filters and keeps its own.
        let mut block = String::new();
        let mut wired = Vec::new();
        let mut unwired = Vec::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some(label) = trimmed
                .strip_prefix('(')
                .and_then(|rest| rest.strip_suffix(')'))
            {
                label.clone_into(&mut block);
            } else if trimmed.contains("query -- ") {
                if trimmed.ends_with(":_repot_repos' \\") {
                    wired.push(block.clone());
                } else {
                    unwired.push(block.clone());
                }
            }
        }
        assert_eq!(wired, ["cd|jump", "info", "open", "rm"]);
        assert_eq!(unwired, ["list"]);
        let path = home.path().join("repot.zsh");
        fs::write(&path, text).expect("completion file");
        if Command::new("zsh").arg("--version").output().is_ok() {
            let loaded = Command::new("zsh")
                .args(["-f", "-c", "source \"$1\"; whence -w _repot_repos"])
                .arg("completion-test")
                .arg(&path)
                .output()
                .expect("zsh runs");
            assert!(
                loaded.status.success(),
                "{}",
                String::from_utf8_lossy(&loaded.stderr)
            );
        }
    }

    #[test]
    fn fish_completes_repository_names_for_query_commands() {
        if Command::new("fish").arg("--version").output().is_err() {
            assert!(
                env::var("REPOT_REQUIRE_SHELLS").as_deref() != Ok("1"),
                "fish is required because REPOT_REQUIRE_SHELLS=1"
            );
            eprintln!("skipping optional fish: unavailable");
            return;
        }
        let home = tempfile::tempdir().expect("temporary home");
        repositories(home.path());
        let completion = write_completions(home.path(), "fish");
        let candidates = |line: &str| -> Vec<String> {
            let output = shell_command("fish", home.path())
                .args(["--no-config", "-c", "source $argv[1]; complete -C $argv[2]"])
                .arg(&completion)
                .arg(line)
                .output()
                .expect("fish runs");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout)
                .expect("completion UTF-8")
                .lines()
                .filter_map(|line| line.split('\t').next())
                .map(str::to_owned)
                .collect()
        };
        let cd = candidates("repot cd al");
        assert!(cd.contains(&"alpha".to_owned()), "{cd:?}");
        assert!(cd.contains(&"alphabet".to_owned()), "{cd:?}");
        assert!(!cd.contains(&"beta".to_owned()), "{cd:?}");
        assert_eq!(candidates("repot jump be"), ["beta"]);
        assert!(candidates("repot info ").contains(&"my repo".to_owned()));
        assert!(candidates("repot status --j").contains(&"--json".to_owned()));
    }
}
