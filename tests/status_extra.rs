//! Safety regressions involving ignored files, Git configuration and shared checkouts.

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    use tempfile::TempDir;

    struct World {
        temp: TempDir,
    }

    impl World {
        fn new() -> Self {
            let world = Self {
                temp: tempfile::tempdir().expect("temporary world"),
            };
            world.git(
                "",
                &["init", "--bare", "--initial-branch=main", "remote.git"],
            );
            world.git("", &["clone", "remote.git", "writer"]);
            fs::write(world.path("writer/base"), "initial\n").expect("base file");
            world.git("writer", &["add", "."]);
            world.git("writer", &["commit", "-m", "initial"]);
            world.git("writer", &["push", "-u", "origin", "main"]);
            world.git("", &["clone", "remote.git", "ghq/project"]);
            world
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.temp.path().join(relative)
        }

        fn command(&self, binary: &str, relative: &str) -> Command {
            let mut command = Command::new(binary);
            command
                .current_dir(self.path(relative))
                .env("HOME", self.temp.path())
                .env("XDG_CONFIG_HOME", self.path("config"))
                .env("XDG_STATE_HOME", self.path("state"))
                .env("GHQ_ROOT", self.path("ghq"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", self.path("gitconfig"))
                .env("GIT_AUTHOR_NAME", "Tester")
                .env("GIT_AUTHOR_EMAIL", "tester@example.invalid")
                .env("GIT_COMMITTER_NAME", "Tester")
                .env("GIT_COMMITTER_EMAIL", "tester@example.invalid")
                .env("GIT_OPTIONAL_LOCKS", "0")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .env_remove("GIT_CONFIG_COUNT");
            if self.path("bin").exists() {
                let inherited = std::env::var_os("PATH").unwrap_or_default();
                let paths =
                    std::iter::once(self.path("bin")).chain(std::env::split_paths(&inherited));
                command
                    .env(
                        "PATH",
                        std::env::join_paths(paths).expect("test executable path"),
                    )
                    .env("REPOT_TEST_GH_RESPONSE", self.path("pr.json"))
                    .env("REPOT_TEST_GH_CALLED", self.path("gh-called"));
            }
            command
        }

        fn git(&self, relative: &str, args: &[&str]) -> String {
            let output = self
                .command("git", relative)
                .args(args)
                .output()
                .expect("run git");
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout)
                .expect("Git UTF-8")
                .trim()
                .to_owned()
        }

        fn repot(&self, args: &[&str]) -> Output {
            self.command(env!("CARGO_BIN_EXE_repot"), "")
                .args(args)
                .output()
                .expect("run repot")
        }

        fn row(output: &Output) -> serde_json::Value {
            let rows: serde_json::Value =
                serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
                    panic!(
                        "JSON: {error}; stdout={} stderr={}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    )
                });
            assert_eq!(rows.as_array().expect("rows").len(), 1);
            rows[0].clone()
        }

        fn advance(&self, name: &str, bytes: &[u8]) {
            fs::write(self.path("writer").join(name), bytes).expect("remote file");
            self.git("writer", &["add", "."]);
            self.git("writer", &["commit", "-m", "remote advance"]);
            self.git("writer", &["push", "origin", "main"]);
            self.git("ghq/project", &["fetch", "--prune", "origin"]);
        }

        fn snapshot(&self) -> (String, Vec<u8>, String) {
            (
                self.git("ghq/project", &["rev-parse", "HEAD"]),
                fs::read(self.path("ghq/project/.git/index")).expect("index bytes"),
                self.git(
                    "ghq/project",
                    &[
                        "for-each-ref",
                        "--format=%(refname) %(objectname)",
                        "refs/heads/",
                        "refs/stash",
                    ],
                ),
            )
        }

        fn assert_review(&self, args: &[&str]) {
            let output = self.repot(args);
            let row = Self::row(&output);
            assert_eq!(row["action"], "review", "{row}");
            assert_eq!(output.status.code(), Some(3), "{row}");
        }

        #[cfg(unix)]
        fn squash_merged_feature(&self) -> String {
            self.git("ghq/project", &["switch", "-c", "feature"]);
            fs::write(
                self.path("ghq/project/feature-file"),
                "feature contribution\n",
            )
            .expect("feature work");
            self.git("ghq/project", &["add", "."]);
            self.git("ghq/project", &["commit", "-m", "feature commit"]);
            self.git("ghq/project", &["push", "-u", "origin", "feature"]);
            let tip = self.git("ghq/project", &["rev-parse", "HEAD"]);
            self.git("writer", &["fetch", "origin"]);
            self.git("writer", &["merge", "--squash", "origin/feature"]);
            self.git("writer", &["commit", "-m", "squash feature"]);
            self.git("writer", &["push", "origin", "main"]);
            self.git("ghq/project", &["fetch", "origin"]);
            // No network call is possible after this URL change: all repot calls
            // use cached refs and the local gh fixture below.
            self.git(
                "ghq/project",
                &[
                    "remote",
                    "set-url",
                    "origin",
                    "https://github.com/example/project",
                ],
            );
            tip
        }

        #[cfg(unix)]
        fn gh_reply(&self, head: &str, base: &str) {
            use std::os::unix::fs::PermissionsExt;

            fs::create_dir(self.path("bin")).expect("stub bin");
            fs::write(
                self.path("pr.json"),
                serde_json::to_vec(&serde_json::json!({
                    "state": "MERGED", "headRefOid": head, "baseRefName": base,
                }))
                .expect("PR JSON"),
            )
            .expect("PR response");
            fs::write(self.path("bin/gh"), "#!/bin/sh\nprintf invoked > \"$REPOT_TEST_GH_CALLED\"\ncat \"$REPOT_TEST_GH_RESPONSE\"\n").expect("gh stub");
            fs::set_permissions(self.path("bin/gh"), fs::Permissions::from_mode(0o755))
                .expect("executable gh stub");
        }
    }

    #[test]
    fn ignored_file_collision_never_overwrites_local_bytes() {
        let world = World::new();
        fs::write(
            world.path("ghq/project/.git/info/exclude"),
            "settings.local\n",
        )
        .expect("local excludes");
        let private = b"local bytes that must survive\0\xff\n";
        fs::write(world.path("ghq/project/settings.local"), private).expect("ignored local file");
        world.advance("settings.local", b"remote tracked version\n");
        let before = world.snapshot();
        world.assert_review(&["sync", "--dry-run", "--no-fetch", "--json"]);
        world.assert_review(&["sync", "--no-fetch", "--json"]);
        assert_eq!(world.snapshot(), before);
        assert_eq!(
            fs::read(world.path("ghq/project/settings.local")).expect("preserved ignored file"),
            private
        );
    }

    #[test]
    fn dirty_sync_with_merge_autostash_never_creates_a_stash() {
        let world = World::new();
        world.git("ghq/project", &["config", "merge.autoStash", "true"]);
        fs::write(world.path("ghq/project/base"), "staged local version\n").expect("staged edit");
        world.git("ghq/project", &["add", "base"]);
        fs::write(world.path("ghq/project/base"), "unstaged local version\n")
            .expect("unstaged edit");
        world.advance("remote-file", b"remote update\n");
        let before = world.snapshot();
        world.assert_review(&["sync", "--json"]);
        assert_eq!(world.snapshot(), before);
        assert_eq!(
            fs::read(world.path("ghq/project/base")).expect("local file"),
            b"unstaged local version\n"
        );
        assert!(world.git("ghq/project", &["stash", "list"]).is_empty());
        assert!(!world.path("ghq/project/.git/MERGE_AUTOSTASH").exists());
    }

    #[test]
    fn populated_submodule_checkout_requires_review() {
        let world = World::new();
        world.git("", &["init", "--initial-branch=main", "dependency"]);
        fs::write(world.path("dependency/data"), "dependency work\n").expect("submodule source");
        world.git("dependency", &["add", "."]);
        world.git("dependency", &["commit", "-m", "dependency"]);
        world.git(
            "writer",
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                path_text(&world.path("dependency")),
                "dependency",
            ],
        );
        world.git("writer", &["commit", "-m", "add submodule"]);
        world.git("writer", &["push", "origin", "main"]);
        world.git("ghq/project", &["fetch", "origin"]);
        world.git("ghq/project", &["merge", "--ff-only", "origin/main"]);
        world.git(
            "ghq/project",
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "update",
                "--init",
            ],
        );
        world.advance("remote-file", b"update above populated submodule\n");
        let before = world.snapshot();
        let dependency_head = world.git("ghq/project/dependency", &["rev-parse", "HEAD"]);
        world.assert_review(&["sync", "--no-fetch", "--json"]);
        assert_eq!(world.snapshot(), before);
        assert_eq!(
            world.git("ghq/project/dependency", &["rev-parse", "HEAD"]),
            dependency_head
        );
        assert_eq!(
            fs::read(world.path("ghq/project/dependency/data")).expect("submodule preserved"),
            b"dependency work\n"
        );
    }

    #[test]
    fn merged_branch_does_not_return_into_an_occupied_default_worktree() {
        let world = World::new();
        world.git("ghq/project", &["switch", "-c", "feature"]);
        world.git("ghq/project", &["push", "-u", "origin", "feature"]);
        world.git(
            "ghq/project",
            &[
                "worktree",
                "add",
                path_text(&world.path("main-worktree")),
                "main",
            ],
        );
        world.advance("remote-file", b"default advance\n");
        let before = world.snapshot();
        let main_before = world.git("main-worktree", &["rev-parse", "HEAD"]);
        world.assert_review(&["sync", "--no-fetch", "--json"]);
        assert_eq!(world.snapshot(), before);
        assert_eq!(
            world.git("ghq/project", &["branch", "--show-current"]),
            "feature"
        );
        assert_eq!(
            world.git("main-worktree", &["rev-parse", "HEAD"]),
            main_before
        );
        assert!(!world.path("main-worktree/remote-file").exists());
    }

    #[test]
    fn shared_current_branch_is_not_updated_under_another_worktree() {
        let world = World::new();
        world.git(
            "ghq/project",
            &[
                "worktree",
                "add",
                "--force",
                path_text(&world.path("shared-main")),
                "main",
            ],
        );
        world.advance("remote-file", b"default advance\n");
        let before = world.snapshot();
        let linked_head = world.git("shared-main", &["rev-parse", "HEAD"]);
        world.assert_review(&["sync", "--no-fetch", "--json"]);
        assert_eq!(world.snapshot(), before);
        assert_eq!(
            world.git("shared-main", &["rev-parse", "HEAD"]),
            linked_head
        );
        assert!(!world.path("shared-main/remote-file").exists());
        assert!(
            world
                .git("shared-main", &["status", "--porcelain"])
                .is_empty()
        );
    }

    #[test]
    fn configured_fetch_refspec_cannot_overwrite_private_local_branch() {
        let world = World::new();
        world.git("ghq/project", &["switch", "-c", "valuable"]);
        fs::write(
            world.path("ghq/project/private-work"),
            "unpublished local commit\n",
        )
        .expect("private file");
        world.git("ghq/project", &["add", "."]);
        world.git("ghq/project", &["commit", "-m", "private local work"]);
        let private_tip = world.git("ghq/project", &["rev-parse", "HEAD"]);
        world.git("ghq/project", &["switch", "main"]);
        world.git("writer", &["switch", "-c", "valuable"]);
        fs::write(
            world.path("writer/different-work"),
            "unrelated remote commit\n",
        )
        .expect("remote file");
        world.git("writer", &["add", "."]);
        world.git("writer", &["commit", "-m", "remote work"]);
        world.git("writer", &["push", "origin", "valuable"]);
        world.git(
            "ghq/project",
            &[
                "config",
                "--replace-all",
                "remote.origin.fetch",
                "+refs/heads/*:refs/heads/*",
            ],
        );
        let before = world.snapshot();
        let output = world.repot(&["status", "--json"]);
        assert_ne!(
            output.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            world.git("ghq/project", &["rev-parse", "valuable"]),
            private_tip
        );
        assert_eq!(world.snapshot(), before);
        assert_eq!(
            world.git("ghq/project", &["show", "valuable:private-work"]),
            "unpublished local commit"
        );
    }

    #[cfg(unix)]
    #[test]
    fn merged_pr_with_exact_local_head_and_default_base_allows_return() {
        let world = World::new();
        let tip = world.squash_merged_feature();
        world.gh_reply(&tip, "main");
        let before = world.snapshot();
        let plan = world.repot(&["sync", "--dry-run", "--no-fetch", "--json"]);
        assert_eq!(World::row(&plan)["action"], "return");
        assert_eq!(world.snapshot(), before);
        assert!(
            world.path("gh-called").exists(),
            "PR evidence must come from the controlled forge"
        );
        let output = world.repot(&["sync", "--no-fetch", "--json"]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            world.git("ghq/project", &["branch", "--show-current"]),
            "main"
        );
        assert_eq!(
            world.git("ghq/project", &["rev-parse", "HEAD"]),
            world.git("writer", &["rev-parse", "HEAD"])
        );
        assert_eq!(
            world.git("ghq/project", &["rev-parse", "feature"]),
            tip,
            "the original feature commits remain reachable"
        );
        assert_eq!(
            fs::read(world.path("ghq/project/feature-file")).expect("merged file"),
            b"feature contribution\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn merged_pr_for_a_different_head_never_leaves_the_local_branch() {
        let world = World::new();
        let merged_tip = world.squash_merged_feature();
        fs::write(
            world.path("ghq/project/after-pr"),
            "work the PR never contained\n",
        )
        .expect("post-PR work");
        world.git("ghq/project", &["add", "."]);
        world.git(
            "ghq/project",
            &["commit", "-m", "local work after merged PR"],
        );
        world.gh_reply(&merged_tip, "main");
        let before = world.snapshot();
        world.assert_review(&["sync", "--no-fetch", "--json"]);
        assert!(world.path("gh-called").exists());
        assert_eq!(world.snapshot(), before);
        assert_eq!(
            world.git("ghq/project", &["branch", "--show-current"]),
            "feature"
        );
    }

    #[cfg(unix)]
    #[test]
    fn merged_pr_into_a_different_base_never_returns_to_main() {
        let world = World::new();
        let tip = world.squash_merged_feature();
        world.gh_reply(&tip, "release");
        let before = world.snapshot();
        let output = world.repot(&["sync", "--no-fetch", "--json"]);
        assert_eq!(World::row(&output)["action"], "none");
        assert_eq!(output.status.code(), Some(0));
        assert!(world.path("gh-called").exists());
        assert_eq!(world.snapshot(), before);
        assert_eq!(
            world.git("ghq/project", &["branch", "--show-current"]),
            "feature"
        );
    }

    fn path_text(path: &Path) -> &str {
        path.to_str().expect("UTF-8 fixture path")
    }
}
