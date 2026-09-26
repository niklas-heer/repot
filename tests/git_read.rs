//! Differential checks for the narrow native layout reader and its public safety guard.

#[path = "../src/git_read.rs"]
mod git_read;

#[cfg(test)]
mod tests {
    use super::git_read::{Layout, OPERATION_MARKERS};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use tempfile::TempDir;

    struct Fixture {
        home: TempDir,
        repo: PathBuf,
    }

    impl Fixture {
        fn new(extra: &[&str]) -> Self {
            let home = tempfile::tempdir().expect("home");
            let repo = home.path().join("projects/host/owner/repo");
            fs::create_dir_all(&repo).expect("tree");
            let fixture = Self { home, repo };
            let mut args = vec!["init", "--initial-branch=main"];
            args.extend_from_slice(extra);
            fixture.git(&fixture.repo, &args);
            fixture.git(&fixture.repo, &["commit", "--allow-empty", "-m", "initial"]);
            fixture.git(
                &fixture.repo,
                &["remote", "add", "origin", "/unused-local-remote"],
            );
            fixture
        }

        fn command(&self, program: &str) -> Command {
            let mut command = Command::new(program);
            command
                .current_dir(self.home.path())
                .env("HOME", self.home.path())
                .env("XDG_CONFIG_HOME", self.home.path().join("config"))
                .env("GHQ_ROOT", self.home.path().join("projects"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", self.home.path().join("gitconfig"))
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
                .env_remove("REPOT_CD_FILE");
            command
        }

        fn git(&self, path: &Path, args: &[&str]) -> String {
            let output = self
                .command("git")
                .current_dir(path)
                .args(args)
                .output()
                .expect("git");
            assert!(
                output.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout)
                .expect("UTF-8")
                .strip_suffix('\n')
                .unwrap_or_default()
                .to_owned()
        }

        fn parity(&self, path: &Path) {
            let layout = Layout::open(path).expect("native supported layout");
            assert_eq!(layout.operation_in_progress(), Some(false));
            for marker in OPERATION_MARKERS {
                let expected = self.git(
                    path,
                    &["rev-parse", "--path-format=absolute", "--git-path", marker],
                );
                let marker_path = layout.marker_path(marker);
                fs::create_dir_all(marker_path.parent().expect("parent")).expect("marker parent");
                fs::write(&marker_path, "").expect("marker");
                assert_eq!(
                    marker_path.canonicalize().expect("native path"),
                    PathBuf::from(expected).canonicalize().expect("Git path"),
                    "{marker}"
                );
                assert_eq!(layout.operation_in_progress(), Some(true), "{marker}");
                fs::remove_file(&marker_path).expect("remove marker");
            }
        }

        fn assert_review(&self) {
            let output = self
                .command(env!("CARGO_BIN_EXE_repot"))
                .args(["sync", "--no-fetch", "--dry-run", "--json"])
                .output()
                .expect("repot");
            assert_eq!(
                output.status.code(),
                Some(3),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let reports: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON");
            assert_eq!(reports[0]["action"], "review");
            assert_eq!(reports[0]["reason"], "Git operation in progress");
        }
    }

    #[test]
    fn native_paths_match_git_for_regular_packed_detached_and_sha256_repositories() {
        for format in ["sha1", "sha256"] {
            let fixture = Fixture::new(&[&format!("--object-format={format}")]);
            fixture.parity(&fixture.repo);
            fixture.git(&fixture.repo, &["pack-refs", "--all"]);
            fixture.parity(&fixture.repo);
            fixture.git(&fixture.repo, &["switch", "--detach"]);
            fixture.parity(&fixture.repo);
        }
    }

    #[test]
    fn native_paths_match_linked_worktrees_and_shared_grafts() {
        let fixture = Fixture::new(&[]);
        let linked = fixture.home.path().join("projects/host/owner/linked\n");
        fixture.git(
            &fixture.repo,
            &[
                "worktree",
                "add",
                "-b",
                "linked",
                linked.to_str().expect("path"),
            ],
        );
        fixture.parity(&linked);
        fs::write(fixture.repo.join(".git/info/grafts"), "").expect("graft marker");
        fixture.assert_review();
        let layout = Layout::open(&linked).expect("layout");
        assert_eq!(layout.operation_in_progress(), Some(true));
    }

    #[test]
    fn native_paths_match_separate_git_directory_and_public_guard() {
        let fixture = Fixture::new(&[]);
        let separate = fixture.home.path().join("private-git-dir");
        fixture.git(
            &fixture.repo,
            &[
                "init",
                "--separate-git-dir",
                separate.to_str().expect("path"),
            ],
        );
        fixture.parity(&fixture.repo);
        fs::create_dir(separate.join("rebase-merge")).expect("operation");
        fixture.assert_review();
    }

    #[test]
    fn unsupported_configuration_is_rejected_and_corrupt_layouts_decline_native_reads() {
        let fixture = Fixture::new(&[]);
        fs::write(
            fixture.repo.join(".git/config"),
            "[core]\nrepositoryformatversion = 1\n[extensions]\nunknownExtension = true\n",
        )
        .expect("unsupported format");
        let output = fixture
            .command(env!("CARGO_BIN_EXE_repot"))
            .args(["status", "--no-fetch", "--json"])
            .output()
            .expect("repot");
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stdout).contains("inspection-failed"));
        fs::write(fixture.repo.join(".git/config"), "not valid config [").expect("invalid config");
        assert!(Layout::open(&fixture.repo).is_none());
        assert!(Layout::open(fixture.home.path()).is_none());
    }
}
