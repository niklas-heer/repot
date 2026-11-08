//! Navigation exercised through the binary and generated shell functions.

#[cfg(test)]
mod tests {

    use std::env;
    use std::fmt::Write as _;
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
                .env("XDG_STATE_HOME", self.directory.path().join("state"))
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

        /// Writes a script or init file into the temporary home and returns its path.
        fn write(&self, name: &str, body: impl AsRef<[u8]>) -> PathBuf {
            let path = self.directory.path().join(name);
            fs::write(&path, body).expect("test file");
            path
        }

        /// A command for `program` with the repot binary first on PATH, so scripts can call it.
        fn shell(&self, program: &str) -> Command {
            let mut command = self.command(program);
            command.env("PATH", repot_path());
            command
        }
    }

    /// Shell tests are mandatory when `REPOT_REQUIRE_SHELLS=1` (set by `mise run ci-native`).
    fn shell_available(shell: &str) -> bool {
        if Command::new(shell).arg("--version").output().is_ok() {
            return true;
        }
        assert!(
            env::var("REPOT_REQUIRE_SHELLS").as_deref() != Ok("1"),
            "{shell} is required because REPOT_REQUIRE_SHELLS=1"
        );
        eprintln!("skipping optional {shell} shell: executable unavailable");
        false
    }

    fn repot_path() -> std::ffi::OsString {
        let binary_dir = Path::new(env!("CARGO_BIN_EXE_repot"))
            .parent()
            .expect("binary directory")
            .to_path_buf();
        env::join_paths(
            std::iter::once(binary_dir)
                .chain(env::split_paths(&env::var_os("PATH").unwrap_or_default())),
        )
        .expect("test PATH")
    }

    /// Points `https://example.test/team/cloned` at an empty local bare remote.
    fn rewrite_example_remote(fixture: &Fixture) -> PathBuf {
        let remote = fixture.directory.path().join("remote");
        assert!(
            fixture
                .command("git")
                .args(["init", "--bare", "--quiet"])
                .arg(&remote)
                .status()
                .expect("bare init")
                .success()
        );
        let rewrite = format!("url.file://{}.insteadOf", remote.display());
        assert!(
            fixture
                .command("git")
                .args([
                    "config",
                    "--global",
                    &rewrite,
                    "https://example.test/team/cloned"
                ])
                .status()
                .expect("URL rewrite")
                .success()
        );
        remote
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

    fn visits(fixture: &Fixture) -> Vec<String> {
        fs::read_to_string(fixture.directory.path().join("state/repot/visits"))
            .unwrap_or_default()
            .lines()
            .map(|line| line.split_once('\t').expect("time and path").1.to_owned())
            .collect()
    }

    #[test]
    fn visits_record_checkout_switches_only() {
        let fixture = Fixture::new();
        let alpha = fixture.repo("host/owner/alpha");
        let beta = fixture.repo("host/owner/beta");
        fs::create_dir_all(beta.join("src/deep")).expect("nested directory");
        let outside = fixture.directory.path().join("tmp");
        for path in [
            &alpha,
            &alpha,
            &beta.join("src/deep"),
            &beta,
            &outside,
            &fixture.directory.path().join("missing"),
        ] {
            let output = fixture.repot(&["visit", "--", path.to_str().expect("path")]);
            assert!(output.status.success(), "visits never fail the shell hook");
            assert!(output.stdout.is_empty() && output.stderr.is_empty());
        }
        // Repeats and moves inside the same checkout are one visit; paths
        // outside any checkout are not recorded at all.
        assert_eq!(
            visits(&fixture),
            [alpha.to_string_lossy(), beta.to_string_lossy()]
        );
        assert!(fixture.repot(&["cd", "alpha"]).status.success());
        assert_eq!(visits(&fixture).len(), 3, "cd counts the jump");
        assert!(fixture.repot(&["cd", "alpha"]).status.success());
        assert_eq!(visits(&fixture).len(), 3, "staying put is not a switch");
    }

    #[test]
    fn info_reports_local_state_of_the_current_or_named_checkout_without_secrets() {
        let fixture = Fixture::new();
        let repo = fixture.repo("host/owner/sprout");
        fixture.repo("host/owner/other");
        let git = |args: &[&str]| {
            let output = fixture
                .command("git")
                .current_dir(&repo)
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
                .args(args)
                .output()
                .expect("git runs");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        fs::write(repo.join("tracked"), "one").expect("file");
        git(&["add", "tracked"]);
        git(&["commit", "-m", "first commit"]);
        fs::write(repo.join("tracked"), "two").expect("edit");
        fs::write(repo.join("new"), "untracked").expect("untracked");
        git(&[
            "remote",
            "add",
            "origin",
            "https://ghp_secret@example.com/owner/sprout.git",
        ]);
        let named = fixture.repot(&["info", "sprout", "--json"]);
        assert!(
            named.status.success(),
            "{}",
            String::from_utf8_lossy(&named.stderr)
        );
        let current = fixture
            .command(env!("CARGO_BIN_EXE_repot"))
            .current_dir(&repo)
            .args(["info", "--json"])
            .output()
            .expect("repot runs");
        assert_eq!(
            current.stdout, named.stdout,
            "no query means the checkout you are in"
        );
        let text = String::from_utf8(named.stdout).expect("UTF-8");
        assert!(!text.contains("ghp_secret"));
        let info: serde_json::Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(info["remote"], "https://example.com/owner/sprout.git");
        assert_eq!(info["modified"], 1);
        assert_eq!(info["untracked"], 1);
        assert_eq!(info["commits"][0]["subject"], "first commit");
        let human = fixture.repot(&["info", "sprout"]);
        let human = String::from_utf8_lossy(&human.stdout);
        assert!(human.contains("1 modified · 1 untracked"));
        assert!(!human.contains("ghp_secret"));
    }

    #[test]
    fn open_launches_the_editor_or_the_web_page_of_the_branch() {
        let fixture = Fixture::new();
        let repo = fixture.repo("host/owner/sprout");
        let git = |args: &[&str]| {
            let output = fixture
                .command("git")
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
                .current_dir(&repo)
                .args(args)
                .output()
                .expect("git runs");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        let recorded = fixture.directory.path().join("launched");
        let launcher = fixture.directory.path().join("record");
        fs::write(
            &launcher,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$REPOT_TEST_LAUNCHED\"\n",
        )
        .expect("launcher");
        fs::set_permissions(
            &launcher,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .expect("executable");
        let open = |args: &[&str]| {
            fixture
                .command(env!("CARGO_BIN_EXE_repot"))
                .env("REPOT_TEST_LAUNCHED", &recorded)
                .env(
                    "REPOT_EDITOR",
                    format!("{} --new-window", launcher.display()),
                )
                .env("BROWSER", &launcher)
                .env_remove("VISUAL")
                .env_remove("EDITOR")
                .args(args)
                .output()
                .expect("repot runs")
        };
        let output = open(&["open", "sprout"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read_to_string(&recorded).expect("launched"),
            format!("--new-window\n{}\n", repo.display())
        );
        let without_remote = open(&["open", "sprout", "--web"]);
        assert!(!without_remote.status.success());
        assert!(String::from_utf8_lossy(&without_remote.stderr).contains("repot publish"));
        git(&["commit", "--allow-empty", "-m", "base"]);
        git(&["branch", "-M", "main"]);
        git(&[
            "remote",
            "add",
            "origin",
            "https://token@github.com/owner/sprout.git",
        ]);
        git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        git(&[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ]);
        assert!(open(&["open", "sprout", "--web"]).status.success());
        assert_eq!(
            fs::read_to_string(&recorded).expect("launched"),
            "https://github.com/owner/sprout\n"
        );
        git(&["switch", "--quiet", "--create", "feature"]);
        assert!(open(&["open", "sprout", "--web"]).status.success());
        assert_eq!(
            fs::read_to_string(&recorded).expect("launched"),
            "https://github.com/owner/sprout/tree/feature\n"
        );
    }

    #[test]
    fn shell_hooks_track_directory_changes_unless_disabled() {
        let fixture = Fixture::new();
        let alpha = fixture.repo("host/owner/alpha");
        let beta = fixture.repo("host/owner/beta");
        // zsh and fish fire their hooks in scripts; bash and Nushell hooks run
        // at the interactive prompt and are covered by their own init scripts.
        for shell in ["zsh", "fish"] {
            if !shell_available(shell) {
                continue;
            }
            let _ = fs::remove_dir_all(fixture.directory.path().join("state"));
            for (flags, expected) in [(&[][..], 2), (&["--no-track"][..], 0)] {
                let init = fixture
                    .repot(&[&["shell-init", shell][..], flags].concat())
                    .stdout;
                let script = fixture.directory.path().join("hook-script");
                let mut body = String::from_utf8(init).expect("UTF-8 init");
                write!(
                    body,
                    "\ncd '{}'\ncd '{}'\ncd '{}'\n",
                    alpha.display(),
                    beta.display(),
                    fixture.directory.path().display()
                )
                .expect("script");
                fs::write(&script, body).expect("hook script");
                let mut command = fixture.command(shell);
                if shell == "zsh" {
                    command.arg("-f");
                } else {
                    command.arg("--no-config");
                }
                let binary = Path::new(env!("CARGO_BIN_EXE_repot"))
                    .parent()
                    .expect("binary directory")
                    .to_path_buf();
                let path = env::join_paths(
                    std::iter::once(binary)
                        .chain(env::split_paths(&env::var_os("PATH").unwrap_or_default())),
                )
                .expect("PATH");
                let output = command
                    .env("PATH", path)
                    .arg(&script)
                    .output()
                    .expect("shell");
                assert!(
                    output.status.success(),
                    "{shell}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert_eq!(visits(&fixture).len(), expected, "{shell} {flags:?}");
                let _ = fs::remove_dir_all(fixture.directory.path().join("state"));
            }
        }
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
        if !shell_available(shell) {
            return;
        }
        let fixture = Fixture::new();
        let wanted = fixture.repo("host/owner/space ' quote ; $(false)\n");
        let init = fixture.repot(&["shell-init", shell]);
        assert!(init.status.success());
        let mut script = String::from_utf8(init.stdout).expect("shell script UTF-8");
        script.push_str(match shell {
        "nu" => "\nrepot jump; $env.PWD | save --force $env.REPOT_TEST_RESULT\ntry { repot jump zzzzzzzzzzzzzzzzzzzz } catch { '1' | save --force $env.REPOT_TEST_STATUS }\n",
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

    fn mutation_fixture(fixture: &Fixture) -> String {
        fixture.repo("host/owner/recover");
        let archived = fixture.repot(&["rm", "recover", "--json"]);
        assert!(
            archived.status.success(),
            "{}",
            String::from_utf8_lossy(&archived.stderr)
        );
        let report: serde_json::Value =
            serde_json::from_slice(&archived.stdout).expect("archive report");
        let id = report[0]["id"].as_str().expect("archive ID").to_owned();
        rewrite_example_remote(fixture);
        id
    }

    fn mutation_shell_test(shell: &str) {
        if !shell_available(shell) {
            return;
        }
        let fixture = Fixture::new();
        let id = mutation_fixture(&fixture);
        let init = fixture.repot(&["shell-init", shell]);
        assert!(init.status.success());
        let mut script = String::from_utf8(init.stdout).expect("shell script");
        let commands = [
            ("NEW", "repot new scratchwork".to_owned()),
            (
                "CREATE",
                "repot create example.test/team/created".to_owned(),
            ),
            (
                "GET",
                "repot get --look example.test/team/cloned".to_owned(),
            ),
            ("RESTORE", format!("repot trash restore {id}")),
            ("DRY", "repot new ignored --dry-run".to_owned()),
        ];
        for (name, command) in &commands {
            writeln!(script, "\n{command}").expect("append command");
            if shell == "nu" {
                writeln!(script, "$env.PWD | save --force $env.REPOT_TEST_{name}")
                    .expect("append Nu capture");
            } else {
                writeln!(script, "printf '%s' \"$PWD\" > \"$REPOT_TEST_{name}\"")
                    .expect("append shell capture");
            }
        }
        let script_path = fixture.directory.path().join("mutations-test");
        fs::write(&script_path, script).expect("mutation script");
        let binary_dir = Path::new(env!("CARGO_BIN_EXE_repot"))
            .parent()
            .expect("binary directory");
        let path = env::join_paths(
            std::iter::once(binary_dir.to_path_buf())
                .chain(env::split_paths(&env::var_os("PATH").unwrap_or_default())),
        )
        .expect("test PATH");
        let mut command = fixture.command(shell);
        command.arg(&script_path).env("PATH", path);
        for (name, _) in &commands {
            command.env(
                format!("REPOT_TEST_{name}"),
                fixture.directory.path().join(name),
            );
        }
        let output = command.output().expect("shell mutation sequence");
        assert!(
            output.status.success(),
            "{shell}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        for (name, relative) in [
            ("NEW", "local/scratch/scratchwork"),
            ("CREATE", "example.test/team/created"),
            ("GET", "example.test/team/cloned"),
            ("RESTORE", "host/owner/recover"),
            ("DRY", "host/owner/recover"),
        ] {
            let expected = fixture
                .root
                .join(relative)
                .canonicalize()
                .expect("created checkout");
            assert_eq!(
                fs::read(fixture.directory.path().join(name)).expect("shell directory"),
                expected.as_os_str().as_encoded_bytes(),
                "{shell} {name}"
            );
        }
        assert!(!fixture.root.join("local/scratch/ignored").exists());
        assert_eq!(
            fs::read_dir(fixture.directory.path().join("tmp"))
                .expect("temporary handoffs")
                .count(),
            0
        );
    }

    #[test]
    fn bash_follows_create_clone_and_restore_handoffs() {
        mutation_shell_test("bash");
    }

    #[test]
    fn zsh_follows_create_clone_and_restore_handoffs() {
        mutation_shell_test("zsh");
    }

    #[test]
    fn fish_follows_create_clone_and_restore_handoffs() {
        mutation_shell_test("fish");
    }

    #[test]
    fn nushell_follows_create_clone_and_restore_handoffs() {
        mutation_shell_test("nu");
    }

    /// Writes the nu init and completion files for this repot binary.
    fn nu_files(fixture: &Fixture) -> (PathBuf, PathBuf) {
        let init = fixture.repot(&["shell-init", "nu"]);
        assert!(init.status.success());
        let completions = fixture.repot(&["completions", "nu"]);
        assert!(completions.status.success());
        (
            fixture.write("nu-init.nu", init.stdout),
            fixture.write("nu-completions.nu", completions.stdout),
        )
    }

    fn nu_script(fixture: &Fixture, name: &str, body: &str) -> PathBuf {
        fixture.write(name, body)
    }

    fn nu_run(fixture: &Fixture, script: &Path) -> Output {
        fixture
            .shell("nu")
            .arg("--no-config-file")
            .arg(script)
            .output()
            .expect("nu runs")
    }

    fn assert_success(output: &Output, label: &str) {
        assert!(
            output.status.success(),
            "{label}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn nushell_wrapper_changes_directory_whichever_file_is_sourced_first() {
        if !shell_available("nu") {
            return;
        }
        let fixture = Fixture::new();
        let wanted = fixture.repo("host/owner/alpha");
        let (init, completions) = nu_files(&fixture);
        for (name, first, second) in [
            ("init-first", &init, &completions),
            ("completions-first", &completions, &init),
        ] {
            let result = fixture.directory.path().join(format!("{name}.result"));
            let script = nu_script(
                &fixture,
                &format!("{name}.nu"),
                &format!(
                    "source '{}'\nsource '{}'\nrepot cd alpha\n$env.PWD | save --force '{}'\n",
                    first.display(),
                    second.display(),
                    result.display()
                ),
            );
            let output = nu_run(&fixture, &script);
            assert_success(&output, name);
            assert_eq!(
                fs::read_to_string(&result).expect("directory"),
                wanted.to_string_lossy(),
                "{name}"
            );
        }
    }

    #[test]
    fn nushell_wrapper_recovers_after_a_failed_call_in_a_script_and_with_nu_c() {
        if !shell_available("nu") {
            return;
        }
        let fixture = Fixture::new();
        let wanted = fixture.repo("host/owner/alpha");
        let (init, _) = nu_files(&fixture);
        let result = fixture.directory.path().join("recovered");
        let script = nu_script(
            &fixture,
            "recover.nu",
            &format!(
                "source '{}'\ntry {{ repot cd zzzzzzzzzzzzzzzz }} catch {{ print failed }}\nrepot cd alpha\n$env.PWD | save --force '{}'\n",
                init.display(),
                result.display()
            ),
        );
        let output = nu_run(&fixture, &script);
        assert_success(&output, "script");
        assert!(String::from_utf8_lossy(&output.stdout).contains("failed"));
        assert_eq!(
            fs::read_to_string(&result).expect("directory"),
            wanted.to_string_lossy()
        );

        // With `nu -c` there is no earlier command to supply an exit status.
        fs::remove_file(&result).expect("reset result");
        let command = format!(
            "source '{}'; try {{ repot cd zzzzzzzzzzzzzzzz }} catch {{ print failed }}; repot cd alpha; $env.PWD | save --force '{}'",
            init.display(),
            result.display()
        );
        let output = fixture
            .shell("nu")
            .args(["--no-config-file", "-c", &command])
            .output()
            .expect("nu -c runs");
        assert_success(&output, "nu -c");
        assert_eq!(
            fs::read_to_string(&result).expect("directory"),
            wanted.to_string_lossy()
        );
    }

    #[test]
    fn nushell_returns_output_of_wrapped_and_plain_commands() {
        if !shell_available("nu") {
            return;
        }
        let fixture = Fixture::new();
        fixture.repo("host/owner/alpha");
        rewrite_example_remote(&fixture);
        let (init, _) = nu_files(&fixture);
        let result = fixture.directory.path().join("fetched.json");
        let script = nu_script(
            &fixture,
            "capture.nu",
            &format!(
                "source '{init}'\nlet listed = (repot list --json | from json)\nlet trash = (repot trash list --json | from json)\nlet fetched = (repot get --look --json example.test/team/cloned | from json)\n$fetched | to json | save --force '{result}'\nprint ($listed | length)\nprint ($trash | length)\n",
                init = init.display(),
                result = result.display()
            ),
        );
        let output = nu_run(&fixture, &script);
        assert_success(&output, "capture");
        assert_eq!(String::from_utf8_lossy(&output.stdout), "1\n0\n");
        let fetched = fs::read_to_string(&result).expect("fetched report");
        assert!(fetched.contains("example.test/team/cloned"), "{fetched}");
    }

    #[test]
    fn nushell_reads_exit_status_with_complete() {
        if !shell_available("nu") {
            return;
        }
        let fixture = Fixture::new();
        let (init, _) = nu_files(&fixture);
        let result = fixture.directory.path().join("status");
        let script = nu_script(
            &fixture,
            "complete.nu",
            &format!(
                "source '{}'\nlet report = (repot info zzzzzzzz | complete)\n$report.exit_code | into string | save --force '{}'\n",
                init.display(),
                result.display()
            ),
        );
        let output = nu_run(&fixture, &script);
        assert_success(&output, "complete");
        let status = fs::read_to_string(&result).expect("status");
        assert_ne!(status, "0", "a failing report reports its status");
    }

    #[test]
    fn nushell_routes_by_subcommand_with_global_options_after_it() {
        if !shell_available("nu") {
            return;
        }
        let fixture = Fixture::new();
        let wanted = fixture.repo("host/owner/alpha");
        let (init, _) = nu_files(&fixture);
        // A manifest named like a report command must not change how the call is routed.
        let manifest = fixture.write("status", "");
        let result = fixture.directory.path().join("manifest-result");
        let script = nu_script(
            &fixture,
            "manifest.nu",
            &format!(
                "source '{}'\nrepot cd --manifest '{}' alpha\n$env.PWD | save --force '{}'\n",
                init.display(),
                manifest.display(),
                result.display()
            ),
        );
        let output = nu_run(&fixture, &script);
        assert_success(&output, "manifest");
        assert_eq!(
            fs::read_to_string(&result).expect("directory"),
            wanted.to_string_lossy()
        );
    }

    #[test]
    fn nushell_visit_hook_is_not_added_again_when_reloaded() {
        if !shell_available("nu") {
            return;
        }
        let fixture = Fixture::new();
        let (init, _) = nu_files(&fixture);
        let script = nu_script(
            &fixture,
            "reload.nu",
            &format!(
                "source '{init}'\nsource '{init}'\nprint ($env.config.hooks.env_change.PWD? | default [] | length)\n",
                init = init.display()
            ),
        );
        let output = nu_run(&fixture, &script);
        assert_success(&output, "reload");
        assert_eq!(String::from_utf8_lossy(&output.stdout), "1\n");
    }

    #[test]
    fn bash_visit_hook_passes_the_previous_status_to_prompt_commands() {
        if !shell_available("bash") {
            return;
        }
        let fixture = Fixture::new();
        let init = fixture.repot(&["shell-init", "bash"]);
        assert!(init.status.success());
        let init = fixture.write("bash-init.sh", init.stdout);
        let status = fixture.directory.path().join("prompt-status");
        let listing = fixture.directory.path().join("prompt-listing");
        let prompt = format!("printf \"%s\" \"$?\" > \"{}\"", status.display());
        // Bash 5.1 introduced array PROMPT_COMMAND; older versions only have the string form.
        let arrays = Command::new("bash")
            .args([
                "-c",
                "((BASH_VERSINFO[0] > 5 || (BASH_VERSINFO[0] == 5 && BASH_VERSINFO[1] >= 1)))",
            ])
            .status()
            .expect("bash version")
            .success();
        let mut cases = vec![(
            "string",
            format!(
                "PROMPT_COMMAND='{prompt}'\nsource '{init}'\nsource '{init}'\nprintf '%s' \"$PROMPT_COMMAND\" > '{listing}'\nfalse\neval \"$PROMPT_COMMAND\"\n",
                init = init.display(),
                listing = listing.display()
            ),
        )];
        if arrays {
            cases.push((
                "array",
                format!(
                    "PROMPT_COMMAND=('{prompt}')\nsource '{init}'\nsource '{init}'\nprintf '%s\\n' \"${{PROMPT_COMMAND[@]}}\" > '{listing}'\nfalse\nfor __repot_command in \"${{PROMPT_COMMAND[@]}}\"; do eval \"$__repot_command\"; done\n",
                    init = init.display(),
                    listing = listing.display()
                ),
            ));
        }
        for (label, body) in cases {
            let script = fixture.write(&format!("prompt-{label}.sh"), format!("set -u\n{body}"));
            fs::remove_file(&status).ok();
            fs::remove_file(&listing).ok();
            let output = fixture
                .shell("bash")
                .arg("--noprofile")
                .arg("--norc")
                .arg(&script)
                .output()
                .expect("bash runs");
            assert_success(&output, label);
            assert_eq!(
                fs::read_to_string(&status).expect("status seen by prompt command"),
                "1",
                "{label}: the prompt command sees the status of `false`"
            );
            let listed = fs::read_to_string(&listing).expect("prompt listing");
            assert_eq!(
                listed.matches("__repot_visit").count(),
                1,
                "{label}: repeated init adds the hook once: {listed}"
            );
        }
    }

    #[test]
    fn fish_cd_dash_returns_to_the_directory_before_a_jump() {
        if !shell_available("fish") {
            return;
        }
        let fixture = Fixture::new();
        let wanted = fixture.repo("host/owner/alpha");
        let elsewhere = fixture.directory.path().join("elsewhere");
        fs::create_dir(&elsewhere).expect("starting directory");
        let elsewhere = elsewhere.canonicalize().expect("canonical start");
        let init = fixture.repot(&["shell-init", "fish"]);
        assert!(init.status.success());
        let init = fixture.write("fish-init.fish", init.stdout);
        let jumped = fixture.directory.path().join("jumped");
        let returned = fixture.directory.path().join("returned");
        let script = fixture.write(
            "jump-back.fish",
            format!(
                "source '{}'\ncd '{}'\nrepot cd alpha\npwd > '{}'\ncd -\npwd > '{}'\n",
                init.display(),
                elsewhere.display(),
                jumped.display(),
                returned.display()
            ),
        );
        let output = fixture
            .shell("fish")
            .arg("--no-config")
            .arg(&script)
            .output()
            .expect("fish runs");
        assert_success(&output, "fish");
        assert_eq!(
            fs::read_to_string(&jumped)
                .expect("jumped directory")
                .trim_end(),
            wanted.to_string_lossy()
        );
        assert_eq!(
            fs::read_to_string(&returned)
                .expect("returned directory")
                .trim_end(),
            elsewhere.to_string_lossy()
        );
    }

    #[test]
    fn zsh_and_fish_record_the_checkout_a_shell_starts_in() {
        for shell in ["zsh", "fish"] {
            if !shell_available(shell) {
                continue;
            }
            let fixture = Fixture::new();
            let alpha = fixture.repo("host/owner/alpha");
            let init = fixture.repot(&["shell-init", shell]);
            assert!(init.status.success());
            let script = fixture.write(&format!("start-{shell}"), init.stdout);
            let flag = if shell == "zsh" { "-f" } else { "--no-config" };
            let output = fixture
                .shell(shell)
                .current_dir(&alpha)
                .arg(flag)
                .arg(&script)
                .output()
                .expect("shell runs");
            assert_success(&output, shell);
            assert_eq!(visits(&fixture), [alpha.to_string_lossy()], "{shell}");
        }
    }
}
