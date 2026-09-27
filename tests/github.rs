//! The batched GitHub survey through the binary. A scripted `gh` answers the
//! GraphQL query, and Git's `insteadOf` maps the github.com URL to a local bare
//! remote, so no network is involved.

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    use serde_json::{Value, json};
    use tempfile::TempDir;

    const URL: &str = "https://github.com/owner/repo.git";

    struct World {
        home: TempDir,
        repo: PathBuf,
        source: PathBuf,
    }

    impl World {
        fn new() -> Self {
            let home = tempfile::tempdir().expect("temporary home");
            let world = Self {
                repo: home.path().join("root/github.com/owner/repo"),
                source: home.path().join("source"),
                home,
            };
            fs::create_dir_all(world.path("bin")).expect("fake tools");
            let gh = r#"#!/bin/sh
echo "$*" >> "$GH_LOG"
[ -f "$GH_RESPONSE" ] || exit 1
cat "$GH_RESPONSE"
"#;
            fs::write(world.path("bin/gh"), gh).expect("fake gh");
            fs::set_permissions(world.path("bin/gh"), fs::Permissions::from_mode(0o755))
                .expect("executable");
            let remote = world.path("remote.git");
            fs::write(
                world.path("gitconfig"),
                format!(
                    "[url \"{}\"]\n\tinsteadOf = {URL}\n[init]\n\tdefaultBranch = main\n",
                    remote.display()
                ),
            )
            .expect("gitconfig");
            world.git(
                world.home.path(),
                &["init", "--bare", remote.to_str().expect("path")],
            );
            world.git(
                world.home.path(),
                &["init", world.source.to_str().expect("path")],
            );
            world.commit(&world.source, "base");
            world.git(&world.source, &["remote", "add", "origin", URL]);
            world.git(&world.source, &["push", "--quiet", "origin", "main"]);
            fs::create_dir_all(world.repo.parent().expect("parent")).expect("tree");
            world.git(
                world.home.path(),
                &["clone", "--quiet", URL, world.repo.to_str().expect("path")],
            );
            world
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.home.path().join(relative)
        }

        fn command(&self, program: &str) -> Command {
            let mut command = Command::new(program);
            let path = std::env::join_paths(std::iter::once(self.path("bin")).chain(
                std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
            ))
            .expect("PATH");
            command
                .current_dir(self.home.path())
                .env("PATH", path)
                .env("HOME", self.home.path())
                .env("XDG_CONFIG_HOME", self.path("config"))
                .env("XDG_STATE_HOME", self.path("state"))
                .env("GHQ_ROOT", self.path("root"))
                .env("GH_LOG", self.path("gh.log"))
                .env("GH_RESPONSE", self.path("gh.json"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", self.path("gitconfig"))
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
                .env_remove("REPOT_CD_FILE");
            command
        }

        fn git(&self, cwd: &Path, args: &[&str]) -> String {
            let output = self
                .command("git")
                .current_dir(cwd)
                .args(args)
                .output()
                .expect("git");
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        }

        fn commit(&self, path: &Path, name: &str) -> String {
            fs::write(path.join(name), name).expect("file");
            self.git(path, &["add", "--", name]);
            self.git(path, &["commit", "--quiet", "-m", name]);
            self.git(path, &["rev-parse", "HEAD"])
        }

        fn answer(&self, default_tip: &str, upstream: Option<&str>, merged: &[(&str, &str)]) {
            let nodes: Vec<Value> = merged
                .iter()
                .map(|(oid, base)| json!({"headRefOid": oid, "baseRefName": base}))
                .collect();
            let answer = json!({"data": {"r0": {
                "defaultBranchRef": {"name": "main", "target": {"oid": default_tip}},
                "upstream": upstream.map(|oid| json!({"target": {"oid": oid}})),
                "pullRequests": {"nodes": nodes},
            }}});
            fs::write(self.path("gh.json"), answer.to_string()).expect("response");
        }

        fn repot(&self, args: &[&str]) -> (Output, Vec<Value>) {
            let output = self
                .command(env!("CARGO_BIN_EXE_repot"))
                .args(args)
                .output()
                .expect("repot runs");
            let reports = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
                panic!(
                    "JSON: {error}; stdout={} stderr={}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                )
            });
            (output, reports)
        }
    }

    #[test]
    fn a_checkout_github_confirms_current_is_not_fetched() {
        let world = World::new();
        let local = world.git(&world.repo, &["rev-parse", "origin/main"]);
        // The remote moves, but GitHub (scripted) still reports the old tip:
        // only a skipped fetch can leave the checkout looking synced.
        world.commit(&world.source, "newer");
        world.git(&world.source, &["push", "--quiet", "origin", "main"]);
        world.answer(&local, Some(&local), &[]);
        let (_, reports) = world.repot(&["status", "--json"]);
        assert_eq!(reports[0]["state"], "synced");
        assert_eq!(world.git(&world.repo, &["rev-parse", "origin/main"]), local);
        assert!(
            fs::read_to_string(world.path("gh.log"))
                .expect("gh called")
                .contains("graphql")
        );
    }

    #[test]
    fn a_different_tip_or_a_failed_query_fetches_as_usual() {
        let world = World::new();
        world.commit(&world.source, "newer");
        world.git(&world.source, &["push", "--quiet", "origin", "main"]);
        let remote_tip = world.git(&world.source, &["rev-parse", "HEAD"]);
        world.answer(&remote_tip, Some(&remote_tip), &[]);
        let (_, reports) = world.repot(&["status", "--json"]);
        assert_eq!(reports[0]["state"], "behind");

        let world = World::new();
        world.commit(&world.source, "newer");
        world.git(&world.source, &["push", "--quiet", "origin", "main"]);
        // No response file: gh fails.
        let (_, reports) = world.repot(&["status", "--json"]);
        assert_eq!(reports[0]["state"], "behind");
    }

    #[test]
    fn a_squash_merged_pull_request_reported_by_github_returns_the_branch() {
        let world = World::new();
        world.git(&world.repo, &["switch", "--quiet", "--create", "feature"]);
        let head = world.commit(&world.repo, "feature-work");
        world.git(
            &world.repo,
            &["push", "--quiet", "--set-upstream", "origin", "feature"],
        );
        // GitHub squash-merges the pull request: same content, new commit.
        world.git(&world.source, &["pull", "--quiet", "origin", "main"]);
        fs::write(world.source.join("feature-work"), "feature-work").expect("squash");
        world.git(&world.source, &["add", "feature-work"]);
        world.git(&world.source, &["commit", "--quiet", "-m", "Feature (#1)"]);
        world.git(&world.source, &["push", "--quiet", "origin", "main"]);
        let main = world.git(&world.source, &["rev-parse", "HEAD"]);
        world.git(&world.repo, &["fetch", "--quiet", "origin"]);
        world.answer(&main, Some(&head), &[(&head, "main")]);

        let (_, reports) = world.repot(&["sync", "--json"]);
        assert_eq!(reports[0]["action"], "return", "{reports:?}");
        assert_eq!(reports[0]["applied"], true);
        assert_eq!(
            world.git(&world.repo, &["branch", "--show-current"]),
            "main"
        );
        assert_eq!(world.git(&world.repo, &["rev-parse", "HEAD"]), main);
        // The feature branch and its commit are kept.
        assert_eq!(world.git(&world.repo, &["rev-parse", "feature"]), head);
        let calls = fs::read_to_string(world.path("gh.log")).expect("gh called");
        assert!(
            !calls.contains("pr view"),
            "one query answered everything: {calls}"
        );
    }
}
