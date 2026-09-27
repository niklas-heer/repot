//! `repot stale` through the binary: your own Git activity and visits count,
//! repot's own fast-forwards do not.

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    use serde_json::Value;
    use tempfile::TempDir;

    const LONG_AGO: &str = "2020-01-01T00:00:00Z";

    struct World {
        home: TempDir,
    }

    impl World {
        fn new() -> Self {
            let world = Self {
                home: tempfile::tempdir().expect("temporary home"),
            };
            fs::create_dir_all(world.path("root")).expect("root");
            world
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.home.path().join(relative)
        }

        fn command(&self, program: &str) -> Command {
            let mut command = Command::new(program);
            command
                .current_dir(self.home.path())
                .env("HOME", self.home.path())
                .env("XDG_CONFIG_HOME", self.path("config"))
                .env("XDG_STATE_HOME", self.path("state"))
                .env("GHQ_ROOT", self.path("root"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", self.path("gitconfig"))
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
                .env_remove("REPOT_CD_FILE");
            command
        }

        /// Run Git as if at `date`, which also dates the reflog entries.
        fn git(&self, cwd: &Path, date: Option<&str>, args: &[&str]) {
            let mut command = self.command("git");
            if let Some(date) = date {
                command
                    .env("GIT_COMMITTER_DATE", date)
                    .env("GIT_AUTHOR_DATE", date);
            }
            let output = command
                .current_dir(cwd)
                .args(args)
                .output()
                .expect("git runs");
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        fn repot(&self, args: &[&str]) -> Output {
            self.command(env!("CARGO_BIN_EXE_repot"))
                .args(args)
                .output()
                .expect("repot runs")
        }

        fn stale(&self) -> Vec<String> {
            let output = self.repot(&["stale", "--days", "30", "--json"]);
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let entries: Vec<Value> = serde_json::from_slice(&output.stdout).expect("JSON");
            entries
                .iter()
                .map(|entry| {
                    let path = entry["path"].as_str().expect("path");
                    Path::new(path)
                        .file_name()
                        .expect("name")
                        .to_string_lossy()
                        .into_owned()
                })
                .collect()
        }
    }

    #[test]
    fn stale_counts_your_activity_and_visits_but_not_repot_sync() {
        let world = World::new();
        let source = world.path("source");
        let remote = world.path("remote.git");
        world.git(
            world.home.path(),
            Some(LONG_AGO),
            &[
                "init",
                "--quiet",
                "--initial-branch=main",
                source.to_str().expect("path"),
            ],
        );
        world.git(
            &source,
            Some(LONG_AGO),
            &["commit", "--quiet", "--allow-empty", "-m", "old"],
        );
        world.git(
            world.home.path(),
            Some(LONG_AGO),
            &[
                "clone",
                "--quiet",
                "--bare",
                source.to_str().expect("path"),
                remote.to_str().expect("path"),
            ],
        );
        world.git(
            &source,
            Some(LONG_AGO),
            &["remote", "add", "origin", remote.to_str().expect("path")],
        );
        for name in ["forgotten", "worked-on", "visited", "synced"] {
            let path = world.path(&format!("root/host/owner/{name}"));
            world.git(
                world.home.path(),
                Some(LONG_AGO),
                &[
                    "clone",
                    "--quiet",
                    remote.to_str().expect("path"),
                    path.to_str().expect("path"),
                ],
            );
        }
        let root = world.path("root/host/owner");
        world.git(
            &root.join("worked-on"),
            None,
            &["commit", "--quiet", "--allow-empty", "-m", "today"],
        );
        let visited = root.join("visited").canonicalize().expect("canonical");
        assert!(
            world
                .repot(&["visit", "--", visited.to_str().expect("path")])
                .status
                .success()
        );
        fs::write(root.join("forgotten/notes"), "local only").expect("local work");
        // Upstream moves; repot sync fast-forwards `synced` (and the others
        // that are clean) today, which must not count as touching them.
        world.git(
            &source,
            None,
            &["commit", "--quiet", "--allow-empty", "-m", "new"],
        );
        world.git(&source, None, &["push", "--quiet", "origin", "main"]);
        let sync = world.repot(&["sync", "--json"]);
        assert!(
            !sync.stdout.is_empty(),
            "{}",
            String::from_utf8_lossy(&sync.stderr)
        );

        assert_eq!(world.stale(), ["forgotten", "synced"]);
        let output = world.repot(&["stale", "--days", "30", "--json"]);
        let entries: Vec<Value> = serde_json::from_slice(&output.stdout).expect("JSON");
        assert_eq!(entries[0]["local_work"], "1 changed file");
        assert!(entries[1]["local_work"].is_null());
    }
}
