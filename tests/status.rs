//! Real-repository status and update safety, through the public binary.

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};
    use tempfile::TempDir;

    struct Fixture {
        home: TempDir,
        repo: PathBuf,
        source: PathBuf,
        remote: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let home = tempfile::tempdir().expect("isolated home");
            let repo = home.path().join("projects/host/owner/repo");
            let source = home.path().join("source");
            let remote = home.path().join("remote.git");
            let fixture = Self {
                home,
                repo,
                source,
                remote,
            };
            fixture.git(
                fixture.home.path(),
                &[
                    "init",
                    "--bare",
                    "--initial-branch=main",
                    fixture.remote.to_str().expect("remote path"),
                ],
            );
            fixture.git(
                fixture.home.path(),
                &[
                    "init",
                    "--initial-branch=main",
                    fixture.source.to_str().expect("source path"),
                ],
            );
            fixture.commit(&fixture.source, "initial", "base");
            fixture.git(
                &fixture.source,
                &[
                    "remote",
                    "add",
                    "origin",
                    fixture.remote.to_str().expect("remote path"),
                ],
            );
            fixture.git(
                &fixture.source,
                &["push", "--set-upstream", "origin", "main"],
            );
            fs::create_dir_all(fixture.repo.parent().expect("parent")).expect("tree");
            fixture.git(
                fixture.home.path(),
                &[
                    "clone",
                    fixture.remote.to_str().expect("remote path"),
                    fixture.repo.to_str().expect("repo path"),
                ],
            );
            fixture
        }

        fn command(&self, program: &str) -> Command {
            let mut command = Command::new(program);
            command
                .current_dir(self.home.path())
                .env("HOME", self.home.path())
                .env("XDG_CONFIG_HOME", self.home.path().join("config"))
                .env("XDG_STATE_HOME", self.home.path().join("state"))
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
                .expect("Git runs");
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout)
                .expect("Git output")
                .trim_end()
                .into()
        }

        fn commit(&self, path: &Path, filename: &str, text: &str) {
            fs::write(path.join(filename), text).expect("worktree file");
            self.git(path, &["add", "--", filename]);
            self.git(path, &["commit", "-m", filename]);
        }

        fn run(&self, args: &[&str]) -> (Output, Value) {
            let output = self
                .command(env!("CARGO_BIN_EXE_repot"))
                .args(args)
                .output()
                .expect("repot runs");
            let value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
                panic!(
                    "JSON output: {} / {}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                )
            });
            (output, value)
        }

        fn feature(&self) {
            self.git(&self.repo, &["switch", "-c", "feature"]);
            self.commit(&self.repo, "feature", "feature work");
            self.git(&self.repo, &["push", "-u", "origin", "feature"]);
        }

        fn merge_feature(&self) {
            self.git(&self.source, &["fetch", "origin"]);
            self.git(&self.source, &["merge", "--ff-only", "origin/feature"]);
            self.git(&self.source, &["push", "origin", "main"]);
        }
    }

    #[test]
    fn status_states_follow_remote_head_and_upstream_precedence() {
        let f = Fixture::new();
        let (_, status) = f.run(&["status", "--no-fetch", "--json"]);
        assert_eq!(status[0]["state"], "synced");
        f.git(&f.repo, &["remote", "remove", "origin"]);
        let (_, status) = f.run(&["status", "--no-fetch", "--json"]);
        assert_eq!(status[0]["state"], "no-remote");
        f.git(
            &f.repo,
            &[
                "remote",
                "add",
                "origin",
                f.remote.to_str().expect("remote"),
            ],
        );
        let (_, status) = f.run(&["status", "--json"]);
        assert_eq!(status[0]["state"], "no-upstream");
        f.git(&f.repo, &["branch", "--set-upstream-to", "origin/main"]);
        f.git(&f.repo, &["switch", "--detach"]);
        let (_, status) = f.run(&["status", "--no-fetch", "--json"]);
        assert_eq!(status[0]["state"], "detached");
        f.git(&f.repo, &["switch", "main"]);
        f.commit(&f.repo, "local", "local commit");
        let (_, status) = f.run(&["status", "--no-fetch", "--json"]);
        assert_eq!(status[0]["state"], "ahead");
        f.commit(&f.source, "remote", "remote commit");
        f.git(&f.source, &["push", "origin", "main"]);
        let (output, status) = f.run(&["status", "--json"]);
        assert_eq!(output.status.code(), Some(3));
        assert_eq!(status[0]["state"], "diverged");
        assert_eq!(status[0]["ahead"], 1);
        assert_eq!(status[0]["behind"], 1);
    }

    #[test]
    fn reports_behind_and_fast_forwards_without_autostash() {
        let f = Fixture::new();
        let before = f.git(&f.repo, &["rev-parse", "HEAD"]);
        f.commit(&f.source, "remote", "remote work");
        f.git(&f.source, &["push", "origin", "main"]);
        let (output, status) = f.run(&["status", "--json"]);
        assert!(output.status.success());
        assert_eq!(status[0]["state"], "behind");
        assert_eq!(status[0]["action"], "pull");
        f.git(&f.repo, &["config", "merge.autoStash", "true"]);
        let (output, status) = f.run(&["sync", "--json"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(status[0]["applied"], true);
        assert_ne!(f.git(&f.repo, &["rev-parse", "HEAD"]), before);
        assert_eq!(
            fs::read_to_string(f.repo.join("remote")).expect("pulled file"),
            "remote work"
        );
        assert_eq!(f.git(&f.repo, &["stash", "list"]), "");
    }

    #[test]
    fn dirty_counts_handle_renames_and_newlines_and_sync_preserves_files() {
        let f = Fixture::new();
        f.git(&f.repo, &["mv", "initial", "renamed\nfile"]);
        fs::write(f.repo.join("renamed\nfile"), "unstaged").expect("edit");
        fs::write(f.repo.join("untracked\nfile"), "untracked").expect("untracked");
        let index = fs::read(f.repo.join(".git/index")).expect("index");
        let head = f.git(&f.repo, &["rev-parse", "HEAD"]);
        let (output, status) = f.run(&["sync", "--no-fetch", "--json"]);
        assert_eq!(output.status.code(), Some(3));
        assert_eq!(
            status[0]["dirty"],
            serde_json::json!({"staged":1,"unstaged":1,"untracked":1})
        );
        assert_eq!(fs::read(f.repo.join(".git/index")).expect("index"), index);
        assert_eq!(f.git(&f.repo, &["rev-parse", "HEAD"]), head);
        assert_eq!(
            fs::read_to_string(f.repo.join("renamed\nfile")).expect("edit"),
            "unstaged"
        );
    }

    #[test]
    fn returns_merged_feature_and_preserves_its_branch() {
        let f = Fixture::new();
        f.feature();
        let feature = f.git(&f.repo, &["rev-parse", "HEAD"]);
        f.merge_feature();
        let (output, status) = f.run(&["sync", "--json"]);
        assert!(output.status.success());
        assert_eq!(status[0]["action"], "return");
        assert_eq!(f.git(&f.repo, &["branch", "--show-current"]), "main");
        assert_eq!(f.git(&f.repo, &["rev-parse", "feature"]), feature);
    }

    #[test]
    fn return_creates_missing_default_with_upstream_tracking() {
        let f = Fixture::new();
        f.feature();
        f.git(&f.repo, &["branch", "-D", "main"]);
        f.merge_feature();
        let (output, _) = f.run(&["sync", "--json"]);
        assert!(output.status.success());
        assert_eq!(
            f.git(&f.repo, &["rev-parse", "--abbrev-ref", "@{upstream}"]),
            "origin/main"
        );
        let (output, status) = f.run(&["status", "--no-fetch", "--json"]);
        assert!(output.status.success());
        assert_eq!(status[0]["state"], "synced");
    }

    #[test]
    fn default_local_commits_and_other_worktree_block_return() {
        let f = Fixture::new();
        f.feature();
        f.merge_feature();
        let feature = f.git(&f.repo, &["rev-parse", "HEAD"]);
        f.git(&f.repo, &["switch", "main"]);
        f.commit(&f.repo, "local", "local main work");
        let local_main = f.git(&f.repo, &["rev-parse", "HEAD"]);
        f.git(&f.repo, &["switch", "feature"]);
        let (output, status) = f.run(&["sync", "--json"]);
        assert_eq!(output.status.code(), Some(3));
        assert_eq!(status[0]["action"], "review");
        assert_eq!(f.git(&f.repo, &["rev-parse", "HEAD"]), feature);
        assert_eq!(f.git(&f.repo, &["rev-parse", "main"]), local_main);
        let other = Fixture::new();
        other.feature();
        other.merge_feature();
        other.git(
            &other.repo,
            &[
                "worktree",
                "add",
                other.home.path().join("other").to_str().expect("worktree"),
                "main",
            ],
        );
        let (output, status) = other.run(&["sync", "--json"]);
        assert_eq!(output.status.code(), Some(3));
        assert!(
            status[0]["reason"]
                .as_str()
                .expect("reason")
                .contains("worktree")
        );
    }

    #[test]
    fn deleted_upstream_with_patch_equivalence_returns_safely() {
        let f = Fixture::new();
        f.feature();
        f.git(&f.source, &["fetch", "origin"]);
        f.commit(&f.source, "independent", "default moved");
        f.git(&f.source, &["cherry-pick", "origin/feature"]);
        f.git(&f.source, &["push", "origin", "main", ":feature"]);
        let (output, status) = f.run(&["sync", "--json"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(status[0]["action"], "return");
        assert_eq!(f.git(&f.repo, &["branch", "--show-current"]), "main");
    }

    #[test]
    fn dry_run_never_fetches_and_preserves_local_and_remote_refs() {
        let f = Fixture::new();
        f.commit(&f.source, "new", "unfetched");
        f.git(&f.source, &["push", "origin", "main"]);
        let refs = f.git(&f.repo, &["show-ref"]);
        let (output, status) = f.run(&["sync", "--dry-run", "--json"]);
        assert!(output.status.success());
        assert_eq!(status[0]["action"], "none");
        assert!(String::from_utf8_lossy(&output.stderr).contains("cached"));
        assert_eq!(f.git(&f.repo, &["show-ref"]), refs);
        assert!(!f.repo.join("new").exists());
    }

    #[test]
    fn operation_markers_submodules_and_stashes_need_review() {
        let f = Fixture::new();
        fs::create_dir(f.repo.join(".git/rebase-merge")).expect("operation marker");
        let (output, status) = f.run(&["sync", "--no-fetch", "--json"]);
        assert_eq!(output.status.code(), Some(3));
        assert!(
            status[0]["reason"]
                .as_str()
                .expect("reason")
                .contains("operation")
        );
        fs::remove_dir(f.repo.join(".git/rebase-merge")).expect("remove marker");
        fs::write(f.repo.join("initial"), "stash me").expect("edit");
        f.git(&f.repo, &["stash", "push"]);
        let (output, status) = f.run(&["status", "--no-fetch", "--json"]);
        assert_eq!(output.status.code(), Some(3));
        assert_eq!(status[0]["stashes"], 1);
        let head = f.git(&f.repo, &["rev-parse", "HEAD"]);
        f.git(
            &f.repo,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{head},sub"),
            ],
        );
        f.git(&f.repo, &["commit", "-m", "submodule"]);
        fs::create_dir(f.repo.join("sub")).expect("uninitialized submodule directory");
        let (output, status) = f.run(&["sync", "--no-fetch", "--json"]);
        assert_eq!(output.status.code(), Some(3));
        assert!(
            status[0]["reason"]
                .as_str()
                .expect("reason")
                .contains("submodule")
        );
    }

    #[test]
    fn ignored_file_collision_is_never_overwritten() {
        let f = Fixture::new();
        fs::write(f.repo.join(".git/info/exclude"), "valuable\n").expect("ignore file");
        fs::write(f.repo.join("valuable"), "my ignored work").expect("ignored work");
        f.commit(&f.source, "valuable", "remote content");
        f.git(&f.source, &["push", "origin", "main"]);
        let head = f.git(&f.repo, &["rev-parse", "HEAD"]);
        let (output, _) = f.run(&["sync", "--json"]);
        assert_eq!(output.status.code(), Some(3));
        assert_eq!(
            fs::read_to_string(f.repo.join("valuable")).expect("ignored work"),
            "my ignored work"
        );
        assert_eq!(f.git(&f.repo, &["rev-parse", "HEAD"]), head);
    }

    #[test]
    fn configured_fetch_refspec_cannot_overwrite_local_branch() {
        let f = Fixture::new();
        f.commit(&f.repo, "local", "local-only commit");
        let head = f.git(&f.repo, &["rev-parse", "HEAD"]);
        f.git(
            &f.repo,
            &[
                "config",
                "remote.origin.fetch",
                "+refs/heads/*:refs/heads/*",
            ],
        );
        f.commit(&f.source, "remote", "new remote commit");
        f.git(&f.source, &["push", "origin", "main"]);
        let (output, status) = f.run(&["sync", "--json"]);
        assert_eq!(output.status.code(), Some(3));
        assert_eq!(status[0]["action"], "review");
        assert_eq!(f.git(&f.repo, &["rev-parse", "HEAD"]), head);
    }
}
