//! Regressions through the binary: credentials in displayed remotes, the shell
//! handoff file in child processes, quoted editor commands, forge branch links
//! and path-based selection.

#![cfg(unix)]

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    use tempfile::TempDir;

    const REPOT: &str = env!("CARGO_BIN_EXE_repot");

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

        fn command(&self, program: impl AsRef<OsStr>) -> Command {
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
                .env_remove("REPOT_CD_FILE")
                .env_remove("REPOT_EDITOR")
                .env_remove("VISUAL")
                .env_remove("EDITOR")
                .env_remove("BROWSER");
            command
        }

        fn repot(&self, args: &[&str]) -> Output {
            self.command(REPOT).args(args).output().expect("repot runs")
        }

        fn git(&self, path: &Path, args: &[&str]) {
            let output = self
                .command("git")
                .arg("-C")
                .arg(path)
                .args(args)
                .output()
                .expect("git runs");
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        fn repo(&self, name: &str) -> PathBuf {
            let path = self.root.join(name);
            fs::create_dir_all(&path).expect("checkout directory");
            self.git(&path, &["init", "--quiet"]);
            path.canonicalize().expect("canonical checkout path")
        }

        fn record(&self) -> PathBuf {
            self.directory.path().join("record")
        }
    }

    /// Writes an executable POSIX shell script at `path`.
    fn script(path: &Path, body: &str) {
        fs::create_dir_all(path.parent().expect("script directory")).expect("script directory");
        fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("script");
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("executable");
    }

    fn stdout(output: &Output) -> String {
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn stderr(output: &Output) -> String {
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    #[test]
    fn info_never_shows_credentials_from_remote_urls() {
        let fixture = Fixture::new();
        let cases = [
            (
                "query",
                "https://example.invalid/team/project.git?access_token=FAKE",
            ),
            (
                "userinfo",
                "https://x-access-token:FAKE@example.invalid/team/project.git",
            ),
            (
                "oauth",
                "https://oauth2:FAKE@example.invalid/team/project.git#FAKE",
            ),
            (
                "token-user",
                "https://FAKE@example.invalid/team/project.git",
            ),
            ("scp", "git@github.com:owner/repo.git"),
        ];
        for (label, url) in cases {
            let path = fixture.repo(&format!("host/owner/{label}"));
            fixture.git(&path, &["remote", "add", "origin", url]);
            let json = fixture.repot(&["info", "--json", label]);
            assert!(json.status.success(), "{}", stderr(&json));
            let json_text = stdout(&json);
            assert!(!json_text.contains("FAKE"), "{json_text}");
            let human = fixture.repot(&["info", label]);
            assert!(human.status.success(), "{}", stderr(&human));
            assert!(!stdout(&human).contains("FAKE"), "{}", stdout(&human));
            let expected = if label == "scp" {
                url.to_owned()
            } else {
                "https://example.invalid/team/project.git".to_owned()
            };
            assert!(
                json_text.contains(&format!("\"remote\": \"{expected}\"")),
                "{json_text}"
            );
            assert!(stdout(&human).contains(&expected), "{}", stdout(&human));
        }
    }

    #[test]
    fn editors_and_browsers_never_inherit_the_shell_handoff_file() {
        let fixture = Fixture::new();
        fixture.repo("host/owner/editable");
        let editor = fixture.directory.path().join("bin/editor");
        script(
            &editor,
            &format!(
                "printf '%s' \"${{REPOT_CD_FILE-unset}}\" > '{}'",
                fixture.record().display()
            ),
        );
        let output = fixture
            .command(REPOT)
            .args(["open", "editable"])
            .env("REPOT_EDITOR", &editor)
            .env("REPOT_CD_FILE", fixture.directory.path().join("handoff"))
            .output()
            .expect("repot runs");
        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(
            fs::read_to_string(fixture.record()).expect("record"),
            "unset"
        );
        assert!(stdout(&output).is_empty());
    }

    #[test]
    fn quoted_editor_commands_keep_spaces_in_the_program_path() {
        let fixture = Fixture::new();
        let repo = fixture.repo("host/owner/quoted");
        let editor = fixture.directory.path().join("editor dir/my editor");
        script(
            &editor,
            &format!("printf '%s\\n' \"$@\" > '{}'", fixture.record().display()),
        );
        let command = format!("\"{}\" --wait 'two words'", editor.display());
        let output = fixture
            .command(REPOT)
            .args(["open", "quoted"])
            .env("REPOT_EDITOR", command)
            .output()
            .expect("repot runs");
        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(
            fs::read_to_string(fixture.record()).expect("record"),
            format!("--wait\ntwo words\n{}\n", repo.display())
        );

        let output = fixture
            .command(REPOT)
            .args(["open", "quoted"])
            .env("REPOT_EDITOR", "\"unterminated")
            .output()
            .expect("repot runs");
        assert!(!output.status.success());
        assert!(stderr(&output).contains("REPOT_EDITOR has an unterminated quote"));
    }

    #[test]
    fn web_links_encode_branch_names_segment_by_segment() {
        let fixture = Fixture::new();
        let repo = fixture.repo("github.com/owner/web");
        fixture.git(
            &repo,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/owner/repo.git",
            ],
        );
        fixture.git(
            &repo,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        );
        fixture.git(
            &repo,
            &["symbolic-ref", "HEAD", "refs/heads/feat#1%25/ünï/x"],
        );
        let browser = fixture.directory.path().join("bin/browser");
        script(
            &browser,
            &format!(
                "printf '%s\\n%s' \"$1\" \"${{REPOT_CD_FILE-unset}}\" > '{}'",
                fixture.record().display()
            ),
        );
        let output = fixture
            .command(REPOT)
            .args(["open", "--web", "web"])
            .env("BROWSER", &browser)
            .env("REPOT_CD_FILE", fixture.directory.path().join("handoff"))
            .output()
            .expect("repot runs");
        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(
            fs::read_to_string(fixture.record()).expect("record"),
            "https://github.com/owner/repo/tree/feat%231%2525/%C3%BCn%C3%AF/x\nunset"
        );
    }

    #[test]
    fn owner_name_and_host_paths_select_one_repository_without_a_terminal() {
        let fixture = Fixture::new();
        let github = fixture.repo("github.com/owner/name");
        let other_host = fixture.repo("git.invalid/owner/name");
        let other_owner = fixture.repo("git.invalid/owner2/name");
        for (query, expected) in [
            ("github.com/owner/name", &github),
            ("git.invalid/owner/name", &other_host),
            ("owner2/name", &other_owner),
        ] {
            let output = fixture.repot(&["cd", query]);
            assert!(output.status.success(), "{query}: {}", stderr(&output));
            assert_eq!(stdout(&output).trim(), expected.to_string_lossy());
        }
        let output = fixture.repot(&["info", "--json", "owner2/name"]);
        assert!(output.status.success(), "{}", stderr(&output));
        assert!(stdout(&output).contains("git.invalid/owner2/name"));

        // Two repositories share `owner/name`, so scripts get an error, not a guess.
        let output = fixture.repot(&["cd", "owner/name"]);
        assert!(!output.status.success());
        assert!(
            stderr(&output).contains("repositories match"),
            "{}",
            stderr(&output)
        );
    }
}
