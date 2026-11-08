//! Selecting repositories for status and sync, through the public binary.

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};
    use tempfile::TempDir;

    /// A ghq-style tree with one checkout per name, each tracking its own local
    /// bare remote. Nothing here touches the real home directory or GitHub.
    struct World {
        home: TempDir,
        names: Vec<String>,
    }

    impl World {
        fn new(names: &[&str]) -> Self {
            let world = Self {
                home: tempfile::tempdir().expect("isolated home"),
                names: names.iter().map(|name| (*name).to_owned()).collect(),
            };
            // Background maintenance would outlive the test and hold its pipes open.
            fs::write(
                world.path("gitconfig"),
                "[gc]\n\tauto = 0\n[maintenance]\n\tauto = false\n",
            )
            .expect("isolated Git config");
            let seed = world.path("seed");
            fs::create_dir_all(&seed).expect("seed directory");
            world.git(&seed, &["init", "--initial-branch=main", "."]);
            world.commit(&seed, "base", "base\n");
            for name in names {
                let remote = world.remote(name);
                world.git(
                    world.home.path(),
                    &[
                        "init",
                        "--bare",
                        "--initial-branch=main",
                        remote.to_str().expect("remote path"),
                    ],
                );
                world.git(
                    &seed,
                    &["push", remote.to_str().expect("remote path"), "main"],
                );
                fs::create_dir_all(world.checkout_parent()).expect("tree");
                world.git(
                    world.home.path(),
                    &[
                        "clone",
                        remote.to_str().expect("remote path"),
                        world.checkout(name).to_str().expect("checkout path"),
                    ],
                );
            }
            world
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.home.path().join(relative)
        }

        fn remote(&self, name: &str) -> PathBuf {
            self.path(&format!("remotes/{name}.git"))
        }

        fn checkout_parent(&self) -> PathBuf {
            self.path("ghq/example/team")
        }

        fn checkout(&self, name: &str) -> PathBuf {
            self.checkout_parent().join(name)
        }

        fn command(&self, program: &str, directory: &Path) -> Command {
            let mut command = Command::new(program);
            command
                .current_dir(directory)
                .env("HOME", self.home.path())
                .env("XDG_CONFIG_HOME", self.path("config"))
                .env("XDG_STATE_HOME", self.path("state"))
                .env("GHQ_ROOT", self.path("ghq"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", self.path("gitconfig"))
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("REPOT_CD_FILE");
            command
        }

        fn git(&self, directory: &Path, args: &[&str]) -> String {
            let output = self
                .command("git", directory)
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
                .to_owned()
        }

        fn commit(&self, directory: &Path, filename: &str, text: &str) {
            fs::write(directory.join(filename), text).expect("file");
            self.git(directory, &["add", "--", filename]);
            self.git(directory, &["commit", "-m", filename]);
        }

        /// Push a new commit to a remote without the checkout fetching it yet.
        fn advance(&self, name: &str) {
            let writer = self.path(&format!("writer-{name}"));
            self.git(
                self.home.path(),
                &[
                    "clone",
                    self.remote(name).to_str().expect("remote path"),
                    writer.to_str().expect("writer path"),
                ],
            );
            self.commit(&writer, &format!("{name}-new"), "remote advance\n");
            self.git(&writer, &["push", "origin", "main"]);
        }

        fn head(&self, name: &str) -> String {
            self.git(&self.checkout(name), &["rev-parse", "HEAD"])
        }

        fn remote_head(&self, name: &str) -> String {
            self.git(&self.remote(name), &["rev-parse", "main"])
        }

        fn tracking_ref(&self, name: &str) -> String {
            self.git(
                &self.checkout(name),
                &["rev-parse", "refs/remotes/origin/main"],
            )
        }

        fn repot(&self, directory: &Path, args: &[&str]) -> Output {
            self.command(env!("CARGO_BIN_EXE_repot"), directory)
                .args(args)
                .output()
                .expect("repot runs")
        }

        fn json_rows(output: &Output) -> Vec<Value> {
            let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
                panic!(
                    "JSON: {error}; stdout={} stderr={}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                )
            });
            value.as_array().expect("JSON array").clone()
        }

        fn row_names(&self, rows: &[Value]) -> Vec<String> {
            let mut names: Vec<String> = rows
                .iter()
                .map(|row| {
                    let path = row["path"].as_str().expect("path");
                    self.names
                        .iter()
                        .find(|name| path.ends_with(&format!("/{name}")))
                        .cloned()
                        .unwrap_or_else(|| path.to_owned())
                })
                .collect();
            names.sort();
            names
        }
    }

    #[test]
    fn sync_of_one_selected_repository_leaves_the_others_alone() {
        let world = World::new(&["alpha", "beta"]);
        world.advance("alpha");
        world.advance("beta");
        let beta_before = world.head("beta");

        let output = world.repot(world.home.path(), &["sync", "alpha", "--json"]);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(world.head("alpha"), world.remote_head("alpha"));
        assert_eq!(world.head("beta"), beta_before, "beta must not change");
        let rows = World::json_rows(&output);
        assert_eq!(world.row_names(&rows), ["alpha"]);
        assert_eq!(rows[0]["applied"], true, "{rows:?}");
    }

    #[test]
    fn sync_dot_inside_a_checkout_selects_only_that_checkout() {
        let world = World::new(&["alpha", "beta"]);
        world.advance("alpha");
        world.advance("beta");
        let beta_before = world.head("beta");

        let output = world.repot(&world.checkout("alpha"), &["sync", ".", "--json"]);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(world.head("alpha"), world.remote_head("alpha"));
        assert_eq!(world.head("beta"), beta_before, "beta must not change");
        assert_eq!(world.row_names(&World::json_rows(&output)), ["alpha"]);
    }

    #[test]
    fn a_query_matching_several_repositories_selects_all_of_them() {
        let world = World::new(&["alpha", "beta", "gamma"]);
        world.advance("alpha");
        world.advance("beta");
        world.advance("gamma");

        let output = world.repot(world.home.path(), &["sync", "team", "--json"]);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(world.row_names(&World::json_rows(&output)).len(), 3);
        assert_eq!(world.head("gamma"), world.remote_head("gamma"));
    }

    #[test]
    fn status_with_a_query_lists_only_the_selected_repository() {
        let world = World::new(&["alpha", "beta"]);

        let everything = world.repot(world.home.path(), &["status", "--json"]);
        assert_eq!(
            world.row_names(&World::json_rows(&everything)),
            ["alpha", "beta"]
        );

        let selected = world.repot(world.home.path(), &["status", "alpha", "--json"]);
        assert!(selected.status.success(), "{selected:?}");
        assert_eq!(world.row_names(&World::json_rows(&selected)), ["alpha"]);

        let from_inside = world.repot(&world.checkout("beta"), &["status", ".", "--json"]);
        assert_eq!(world.row_names(&World::json_rows(&from_inside)), ["beta"]);
    }

    #[test]
    fn unmatched_query_fails_with_usage_status_before_anything_is_fetched() {
        let world = World::new(&["alpha", "beta"]);
        world.advance("alpha");
        world.advance("beta");
        let alpha_before = world.head("alpha");
        let beta_tracking = world.tracking_ref("beta");

        for args in [
            vec!["sync", "nowhere", "--json"],
            vec!["status", "nowhere", "--json"],
        ] {
            let output = world.repot(world.home.path(), &args);
            assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
            assert!(output.stdout.is_empty(), "{args:?}");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains("no repository matches"), "{stderr}");
            assert!(stderr.contains("hint:"), "{stderr}");
        }
        // Dot outside any checkout matches nothing either.
        let outside = world.repot(world.home.path(), &["sync", "."]);
        assert_eq!(outside.status.code(), Some(2), "{outside:?}");

        assert_eq!(world.head("alpha"), alpha_before);
        assert_eq!(world.tracking_ref("beta"), beta_tracking, "nothing fetched");
        assert_ne!(world.head("beta"), world.remote_head("beta"));
    }

    #[test]
    fn piped_status_still_lists_every_repository() {
        let names: Vec<String> = (0..30).map(|number| format!("repo{number:02}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let world = World::new(&refs);

        let output = world.repot(world.home.path(), &["status"]);
        assert!(output.status.success(), "{output:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(stdout.lines().count(), 30, "{stdout}");
        assert!(!stdout.contains("--verbose"), "{stdout}");
    }
}
