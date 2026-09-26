//! Navigation exercised through the binary and generated shell functions.

#[cfg(test)]
mod tests {

    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    use tempfile::TempDir;

    struct Fixture {
        directory: TempDir,
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = tempfile::tempdir().expect("temporary home");
            let root = directory.path().join("projects");
            fs::create_dir_all(&root).expect("projects directory");
            fs::create_dir(directory.path().join("tmp")).expect("temporary handoffs");
            Self { directory, root }
        }

        fn command(&self, program: &str) -> Command {
            let mut command = Command::new(program);
            command
                .current_dir(self.directory.path())
                .env("HOME", self.directory.path())
                .env("XDG_CONFIG_HOME", self.directory.path().join("config"))
                .env("GHQ_ROOT", &self.root)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", self.directory.path().join("gitconfig"))
                .env("TMPDIR", self.directory.path().join("tmp"))
                .env_remove("REPOT_CD_FILE");
            command
        }

        fn repo(&self, name: &str) -> PathBuf {
            let path = self.root.join(name);
            fs::create_dir_all(&path).expect("checkout directory");
            let output = self
                .command("git")
                .arg("init")
                .arg(&path)
                .output()
                .expect("git init runs");
            assert!(output.status.success());
            path.canonicalize().expect("canonical checkout path")
        }

        fn repot(&self, args: &[&str]) -> Output {
            self.command(env!("CARGO_BIN_EXE_repot"))
                .args(args)
                .output()
                .expect("repot runs")
        }
    }

    #[test]
    fn exact_names_win_over_fuzzy_matches_and_write_raw_handoff() {
        let fixture = Fixture::new();
        let wanted = fixture.repo("host/owner/sprout");
        fixture.repo("host/owner/sprouting");
        let output = fixture.repot(&["jump", "sprout"]);
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            wanted.to_string_lossy()
        );
        let file = fixture.directory.path().join("handoff");
        let output = fixture
            .command(env!("CARGO_BIN_EXE_repot"))
            .args(["jump", "sprout"])
            .env("REPOT_CD_FILE", &file)
            .output()
            .expect("repot runs");
        assert!(output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(
            fs::read(file).expect("handoff"),
            wanted.as_os_str().as_encoded_bytes()
        );
    }

    #[test]
    fn ambiguity_and_missing_matches_fail_without_changing_handoff() {
        let fixture = Fixture::new();
        fixture.repo("host/owner/one");
        fixture.repo("host/owner/two");
        for args in [vec!["jump"], vec!["jump", "zzznotfound"]] {
            let file = fixture.directory.path().join("handoff");
            fs::write(&file, b"unchanged").expect("existing handoff");
            let output = fixture
                .command(env!("CARGO_BIN_EXE_repot"))
                .args(&args)
                .env("REPOT_CD_FILE", &file)
                .output()
                .expect("repot runs");
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
            assert_eq!(fs::read(&file).expect("handoff"), b"unchanged");
        }
    }

    #[test]
    fn a_unique_fuzzy_match_works_without_a_terminal_or_picker() {
        let fixture = Fixture::new();
        let wanted = fixture.repo("host/owner/sunflower");
        fixture.repo("host/owner/grass");
        let output = fixture.repot(&["jump", "snflwr"]);
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            wanted.to_string_lossy()
        );
    }

    #[test]
    fn invalid_shell_is_rejected() {
        let fixture = Fixture::new();
        let output = fixture.repot(&["shell-init", "made-up-shell"]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }

    fn shell_test(shell: &str) {
        if Command::new(shell).arg("--version").output().is_err() {
            eprintln!("skipping optional {shell} shell: executable unavailable");
            return;
        }
        let fixture = Fixture::new();
        let wanted = fixture.repo("host/owner/space ' quote ; $(false)\n");
        let init = fixture.repot(&["shell-init", shell]);
        assert!(init.status.success());
        let mut script = String::from_utf8(init.stdout).expect("shell script UTF-8");
        script.push_str(match shell {
        "nu" => "\nrepot jump; $env.PWD | save --force $env.REPOT_TEST_RESULT; repot jump zzzzzzzzzzzzzzzzzzzz; $env.LAST_EXIT_CODE | into string | save --force $env.REPOT_TEST_STATUS\n",
        "fish" => "\nrepot jump; printf '%s' \"$PWD\" > \"$REPOT_TEST_RESULT\"; repot jump zzzzzzzzzzzzzzzzzzzz; printf '%s' $status > \"$REPOT_TEST_STATUS\"\n",
        _ => "\nrepot jump; printf '%s' \"$PWD\" > \"$REPOT_TEST_RESULT\"; repot jump zzzzzzzzzzzzzzzzzzzz; printf '%s' \"$?\" > \"$REPOT_TEST_STATUS\"\n",
    });
        let script_path = fixture.directory.path().join("wrapper-test");
        fs::write(&script_path, script).expect("test script");
        let result = fixture.directory.path().join("result");
        let status = fixture.directory.path().join("status");
        let binary_dir = Path::new(env!("CARGO_BIN_EXE_repot"))
            .parent()
            .expect("binary directory");
        let path = env::join_paths(
            std::iter::once(binary_dir.to_path_buf())
                .chain(env::split_paths(&env::var_os("PATH").unwrap_or_default())),
        )
        .expect("test PATH");
        let output = fixture
            .command(shell)
            .arg(&script_path)
            .env("PATH", path)
            .env("REPOT_TEST_RESULT", &result)
            .env("REPOT_TEST_STATUS", &status)
            .output()
            .expect("shell runs");
        assert!(
            output.status.success(),
            "{shell}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read_to_string(&result).expect("shell directory result"),
            wanted.to_string_lossy()
        );
        assert_eq!(
            fs::read_to_string(&status).expect("shell failure status"),
            "1"
        );
        assert_eq!(
            fs::read_dir(fixture.directory.path().join("tmp"))
                .expect("temporary directory")
                .count(),
            0
        );
    }

    #[test]
    fn bash_integration_preserves_paths_status_and_cleans_up() {
        shell_test("bash");
    }

    #[test]
    fn zsh_integration_preserves_paths_status_and_cleans_up() {
        shell_test("zsh");
    }

    #[test]
    fn nushell_integration_preserves_paths_status_and_cleans_up() {
        shell_test("nu");
    }

    #[test]
    fn fish_integration_preserves_paths_status_and_cleans_up() {
        shell_test("fish");
    }
}
