//! Generated completions load in real shells without reading repository config.

#[cfg(test)]
mod tests {
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
}
