//! Seeded action sequences against the real binary, Git index, refs and bare remotes.

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    use tempfile::TempDir;

    #[derive(Clone, Copy, Debug)]
    enum Event {
        RemoteCommit,
        Edit,
        Stage,
        Untracked,
        LocalCommit,
        DeleteMergedUpstream,
        RemoteUnavailable,
        Observe,
    }

    #[derive(Debug, Eq, PartialEq)]
    struct Snapshot {
        refs: String,
        head: String,
        index: Vec<u8>,
        files: BTreeMap<PathBuf, Vec<u8>>,
    }

    struct Simulation {
        temp: TempDir,
        seed: u64,
        trace: Vec<String>,
    }

    impl Drop for Simulation {
        fn drop(&mut self) {
            if std::thread::panicking() {
                eprintln!(
                    "replay seed={} transition trace={:#?}",
                    self.seed, self.trace
                );
            }
        }
    }

    impl Simulation {
        fn new(seed: u64) -> Self {
            let world = Self {
                temp: tempfile::tempdir().expect("simulation directory"),
                seed,
                trace: Vec::new(),
            };
            world.git(
                world.temp.path(),
                &["init", "--bare", "--initial-branch=main", "remote.git"],
            );
            world.git(world.temp.path(), &["clone", "remote.git", "writer"]);
            world.write("writer/base", "initial\n");
            world.git(&world.path("writer"), &["add", "base"]);
            world.git(&world.path("writer"), &["commit", "-m", "initial"]);
            world.git(&world.path("writer"), &["push", "-u", "origin", "main"]);
            world.git(world.temp.path(), &["clone", "remote.git", "ghq/project"]);
            world
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.temp.path().join(relative)
        }

        fn command(&self, program: &str, cwd: &Path) -> Command {
            let mut command = Command::new(program);
            command
                .current_dir(cwd)
                .env("HOME", self.temp.path())
                .env("XDG_CONFIG_HOME", self.path("config"))
                .env("XDG_STATE_HOME", self.path("state"))
                .env("GHQ_ROOT", self.path("ghq"))
                .env("GIT_CONFIG_GLOBAL", self.path("gitconfig"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_NAME", "Simulation")
                .env("GIT_AUTHOR_EMAIL", "simulation@example.invalid")
                .env("GIT_COMMITTER_NAME", "Simulation")
                .env("GIT_COMMITTER_EMAIL", "simulation@example.invalid")
                .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
                .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z")
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_OPTIONAL_LOCKS", "0")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .env_remove("GIT_CONFIG_COUNT");
            command
        }

        fn git(&self, cwd: &Path, args: &[&str]) -> String {
            let output = self
                .command("git", cwd)
                .args(args)
                .output()
                .expect("run git");
            assert!(
                output.status.success(),
                "seed={} trace={:?} git={args:?}: {}",
                self.seed,
                self.trace,
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout)
                .expect("UTF-8 Git output")
                .trim()
                .to_owned()
        }

        fn repot(&self, args: &[&str]) -> Output {
            let output = self
                .command(env!("CARGO_BIN_EXE_repot"), self.temp.path())
                .args(args)
                .output()
                .expect("run repot");
            assert!(
                matches!(output.status.code(), Some(0 | 1 | 3)),
                "seed={} trace={:?} args={args:?}: {}",
                self.seed,
                self.trace,
                String::from_utf8_lossy(&output.stderr)
            );
            output
        }

        fn write(&self, relative: &str, contents: &str) {
            fs::write(self.path(relative), contents).expect("write simulated file");
        }

        fn snapshot(&self) -> Snapshot {
            let checkout = self.path("ghq/project");
            let mut files = BTreeMap::new();
            collect_files(&checkout, &checkout, &mut files);
            Snapshot {
                refs: self.git(&checkout, &["show-ref", "--head"]),
                head: fs::read_to_string(checkout.join(".git/HEAD")).expect("HEAD"),
                index: fs::read(checkout.join(".git/index")).expect("index"),
                files,
            }
        }

        fn event(&self, event: Event, step: usize) {
            let checkout = self.path("ghq/project");
            match event {
                Event::RemoteCommit => {
                    self.write(
                        &format!("writer/remote-{step}"),
                        &format!("remote seed={} step={step}\n", self.seed),
                    );
                    self.git(&self.path("writer"), &["add", "."]);
                    self.git(&self.path("writer"), &["commit", "-m", "remote progress"]);
                    self.git(&self.path("writer"), &["push", "origin", "main"]);
                    self.git(&checkout, &["fetch", "--prune", "origin"]);
                }
                Event::Edit => self.write(
                    "ghq/project/base",
                    &format!("local seed={} step={step}\n", self.seed),
                ),
                Event::Stage => {
                    self.write(&format!("ghq/project/staged-{step}"), "staged work\n");
                    self.git(&checkout, &["add", "."]);
                }
                Event::Untracked => self.write(
                    &format!("ghq/project/untracked-{step}"),
                    "keep this untracked work\n",
                ),
                Event::LocalCommit => {
                    self.git(&checkout, &["add", "."]);
                    self.git(
                        &checkout,
                        &["commit", "--allow-empty", "-m", "local progress"],
                    );
                }
                Event::DeleteMergedUpstream => {
                    self.git(&checkout, &["add", "."]);
                    self.git(
                        &checkout,
                        &["commit", "--allow-empty", "-m", "finish topic"],
                    );
                    let branch = format!("topic-{}-{step}", self.seed);
                    self.git(&checkout, &["switch", "-c", &branch]);
                    self.git(&checkout, &["push", "-u", "origin", &branch]);
                    self.git(&self.path("writer"), &["fetch", "origin"]);
                    self.git(
                        &self.path("writer"),
                        &["merge", "--no-edit", &format!("origin/{branch}")],
                    );
                    self.git(&self.path("writer"), &["push", "origin", "main"]);
                    self.git(
                        &self.path("writer"),
                        &["push", "origin", "--delete", &branch],
                    );
                    self.git(&checkout, &["fetch", "--prune", "origin"]);
                }
                Event::RemoteUnavailable => {
                    let before = self.snapshot();
                    fs::rename(self.path("remote.git"), self.path("offline.git"))
                        .expect("inject unavailable remote");
                    let output = self.repot(&["status", "--json"]);
                    fs::rename(self.path("offline.git"), self.path("remote.git"))
                        .expect("restore remote");
                    let rows: serde_json::Value =
                        serde_json::from_slice(&output.stdout).expect("status JSON");
                    assert_eq!(
                        rows[0]["state"], "fetch-failed",
                        "seed={} trace={:?}",
                        self.seed, self.trace
                    );
                    assert_eq!(
                        output.status.code(),
                        Some(1),
                        "seed={} trace={:?}",
                        self.seed,
                        self.trace
                    );
                    assert_eq!(
                        self.snapshot(),
                        before,
                        "fetch failure changed checkout: seed={} trace={:?}",
                        self.seed,
                        self.trace
                    );
                }
                Event::Observe => {}
            }
        }

        fn verify_sync(&mut self, event: Event, step: usize) {
            let checkout = self.path("ghq/project");
            let before = self.snapshot();
            let old_head = self.git(&checkout, &["rev-parse", "HEAD"]);
            let planned = self.repot(&["sync", "--dry-run", "--no-fetch", "--json"]);
            let rows: serde_json::Value =
                serde_json::from_slice(&planned.stdout).expect("dry-run JSON");
            let action = rows[0]["action"].as_str().expect("planned action");
            self.trace.push(format!("action={action}"));
            if step == 0 {
                assert_eq!(
                    action, "pull",
                    "clean remote progress must fast-forward: seed={} trace={:?}",
                    self.seed, self.trace
                );
            }
            if matches!(event, Event::DeleteMergedUpstream) {
                assert_eq!(
                    action, "return",
                    "merged deleted upstream must return: seed={} trace={:?}",
                    self.seed, self.trace
                );
            }
            if matches!(event, Event::Edit | Event::Stage | Event::Untracked) {
                assert_eq!(
                    action, "review",
                    "dirty work must be reviewed: seed={} trace={:?}",
                    self.seed, self.trace
                );
            }
            assert_eq!(
                self.snapshot(),
                before,
                "dry-run mutated checkout: seed={} trace={:?}",
                self.seed,
                self.trace
            );
            let synced = self.repot(&["sync", "--no-fetch", "--json"]);
            if matches!(action, "pull" | "return") {
                assert_eq!(
                    synced.status.code(),
                    Some(0),
                    "safe sync failed: seed={} trace={:?}: {}",
                    self.seed,
                    self.trace,
                    String::from_utf8_lossy(&synced.stderr)
                );
                self.git(
                    &checkout,
                    &["merge-base", "--is-ancestor", &old_head, "HEAD"],
                );
                let after = self.snapshot();
                for (path, bytes) in &before.files {
                    assert_eq!(
                        after.files.get(path),
                        Some(bytes),
                        "sync lost existing work: seed={} trace={:?} path={path:?}",
                        self.seed,
                        self.trace
                    );
                }
            } else {
                assert_eq!(
                    self.snapshot(),
                    before,
                    "review/none action mutated checkout: seed={} trace={:?}",
                    self.seed,
                    self.trace
                );
                if matches!(action, "review" | "push") {
                    assert_eq!(
                        synced.status.code(),
                        Some(3),
                        "review must be visible: seed={} trace={:?}",
                        self.seed,
                        self.trace
                    );
                }
            }
        }
    }

    fn collect_files(base: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).expect("checkout files") {
            let entry = entry.expect("checkout entry");
            if entry.file_name() == ".git" {
                continue;
            }
            if entry.file_type().expect("entry type").is_dir() {
                collect_files(base, &entry.path(), files);
            } else {
                files.insert(
                    entry
                        .path()
                        .strip_prefix(base)
                        .expect("relative path")
                        .to_path_buf(),
                    fs::read(entry.path()).expect("worktree bytes"),
                );
            }
        }
    }

    fn run_seed(seed: u64) {
        let mut simulation = Simulation::new(seed);
        let mut events = [
            Event::RemoteCommit,
            Event::Edit,
            Event::Stage,
            Event::Untracked,
            Event::LocalCommit,
            Event::RemoteCommit,
            Event::DeleteMergedUpstream,
            Event::RemoteUnavailable,
            Event::Observe,
            Event::RemoteCommit,
            Event::Edit,
            Event::Untracked,
            Event::Stage,
            Event::LocalCommit,
            Event::DeleteMergedUpstream,
            Event::RemoteUnavailable,
        ];
        let mut state = seed;
        // Shuffle the middle workload while preserving a clean initial fast-forward
        // and a final merged-upstream recovery in every reproducible sequence.
        for index in 1..14 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let other = usize::try_from(state % 13)
                .expect("bounded choice")
                .saturating_add(1);
            events.swap(index, other);
        }
        for (step, event) in events.into_iter().enumerate() {
            simulation
                .trace
                .push(format!("step={step} event={event:?}"));
            simulation.event(event, step);
            simulation.verify_sync(event, step);
        }
    }

    #[test]
    fn seeded_workflows_7() {
        run_seed(7);
    }

    #[test]
    fn seeded_workflows_42() {
        run_seed(42);
    }

    #[test]
    fn seeded_workflows_2026() {
        run_seed(2026);
    }

    #[test]
    fn seeded_workflows_65537() {
        run_seed(65_537);
    }
}
