//! Malformed and stuck Git helpers cannot make a checkout appear safe.

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::env;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};
    use std::time::{Duration, Instant};

    use tempfile::TempDir;

    struct Fixture {
        directory: TempDir,
        git: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let git = env::split_paths(&env::var_os("PATH").expect("PATH"))
                .map(|directory| directory.join("git"))
                .find(|path| path.is_file())
                .expect("installed Git");
            let fixture = Self {
                directory: tempfile::tempdir().expect("isolated fixture"),
                git,
            };
            fixture.git(&["init", "--bare", "--initial-branch=main", "remote.git"]);
            fixture.git(&["clone", "remote.git", "ghq/project"]);
            fixture.git(&[
                "-C",
                "ghq/project",
                "commit",
                "--allow-empty",
                "-m",
                "initial",
            ]);
            fixture.git(&["-C", "ghq/project", "push", "-u", "origin", "main"]);
            fixture
        }

        fn command(&self, program: &Path) -> Command {
            let mut command = Command::new(program);
            command
                .current_dir(self.directory.path())
                .env("HOME", self.directory.path())
                .env("XDG_CONFIG_HOME", self.directory.path().join("config"))
                .env("XDG_STATE_HOME", self.directory.path().join("state"))
                .env("GHQ_ROOT", self.directory.path().join("ghq"))
                .env("GIT_CONFIG_GLOBAL", self.directory.path().join("gitconfig"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_NAME", "Safety test")
                .env("GIT_AUTHOR_EMAIL", "safety@example.invalid")
                .env("GIT_COMMITTER_NAME", "Safety test")
                .env("GIT_COMMITTER_EMAIL", "safety@example.invalid")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .env_remove("GIT_CONFIG_COUNT");
            command
        }

        fn git(&self, args: &[&str]) -> String {
            let output = self
                .command(&self.git)
                .args(args)
                .output()
                .expect("Git runs");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).expect("Git stdout")
        }

        fn repot(&self, args: &[&str], interception: &str) -> Output {
            let bin = self.directory.path().join("bin");
            fs::create_dir_all(&bin).expect("stub directory");
            let stub = bin.join("git");
            fs::write(
                &stub,
                format!("#!/bin/sh\n{interception}\nexec \"$REPOT_REAL_GIT\" \"$@\"\n"),
            )
            .expect("Git stub");
            fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).expect("executable stub");
            let path = env::join_paths(
                std::iter::once(bin).chain(env::split_paths(&env::var_os("PATH").expect("PATH"))),
            )
            .expect("stub PATH");
            self.command(Path::new(env!("CARGO_BIN_EXE_repot")))
                .args(args)
                .env("PATH", path)
                .env("REPOT_REAL_GIT", &self.git)
                .env("REPOT_CHILD_PID", self.directory.path().join("child-pid"))
                .output()
                .expect("repot runs")
        }
    }

    #[test]
    fn oversized_status_output_is_an_error_and_never_a_clean_checkout() {
        let fixture = Fixture::new();
        let before = fixture.git(&["-C", "ghq/project", "rev-parse", "HEAD"]);
        let output = fixture.repot(&["sync", "--no-fetch", "--json"],
            "for argument do\n  if [ \"$argument\" = status ]; then\n    head -c 16777217 /dev/zero\n    exit 0\n  fi\ndone");
        assert!(!output.status.success());
        let rows: serde_json::Value = serde_json::from_slice(&output.stdout).expect("status JSON");
        assert_eq!(rows[0]["action"], "review");
        assert_eq!(
            fixture.git(&["-C", "ghq/project", "rev-parse", "HEAD"]),
            before
        );
    }

    #[test]
    fn inherited_git_path_overrides_cannot_redirect_repository_inspection() {
        let fixture = Fixture::new();
        fs::write(
            fixture.directory.path().join("ghq/project/local-work"),
            "keep me\n",
        )
        .expect("untracked local work");
        let invalid = fixture.directory.path().join("nonexistent");
        let output = fixture
            .command(Path::new(env!("CARGO_BIN_EXE_repot")))
            .args(["status", "--no-fetch", "--json"])
            .env("GIT_DIR", &invalid)
            .env("GIT_WORK_TREE", &invalid)
            .env("GIT_INDEX_FILE", &invalid)
            .output()
            .expect("repot runs");
        let rows: serde_json::Value = serde_json::from_slice(&output.stdout).expect("status JSON");
        assert_eq!(rows[0]["state"], "synced");
        assert_eq!(rows[0]["action"], "review");
        assert_eq!(rows[0]["dirty"]["untracked"], 1);
    }

    #[test]
    fn fetch_timeout_terminates_a_descendant_holding_the_output_pipe() {
        let fixture = Fixture::new();
        let started = Instant::now();
        let output = fixture.repot(&["status", "--timeout", "1", "--json"],
            "for argument do\n  if [ \"$argument\" = fetch ]; then\n    sleep 60 &\n    printf '%s' \"$!\" > \"$REPOT_CHILD_PID\"\n    exit 0\n  fi\ndone");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "fetch timeout did not bound inherited pipe lifetime"
        );
        assert_eq!(output.status.code(), Some(1));
        let rows: serde_json::Value = serde_json::from_slice(&output.stdout).expect("status JSON");
        assert_eq!(rows[0]["state"], "fetch-failed");
        assert_eq!(rows[0]["action"], "review");
        let pid = fs::read_to_string(fixture.directory.path().join("child-pid"))
            .expect("spawned child ID");
        let status = Command::new("ps")
            .args(["-o", "stat=", "-p", pid.trim()])
            .output()
            .expect("inspect known test child");
        let state = String::from_utf8_lossy(&status.stdout);
        assert!(
            state.trim().is_empty() || state.trim().starts_with('Z'),
            "helper descendant still running: {state}"
        );
    }
}
