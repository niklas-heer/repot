//! Signals unwind the CLI and terminate Git/helper descendants before returning.

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::env;
    use std::fs;
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    use nix::sys::signal::{Signal, kill, killpg};
    use nix::unistd::Pid;

    struct Running {
        child: Child,
        helper: PathBuf,
    }

    impl Drop for Running {
        fn drop(&mut self) {
            if let Ok(text) = fs::read_to_string(&self.helper)
                && let Ok(pid) = text.parse::<i32>()
            {
                let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn assert_cancelled(signal: Signal) {
        let directory = tempfile::tempdir().expect("isolated HOME");
        let root = directory.path();
        let bin = root.join("bin");
        fs::create_dir(&bin).expect("helper directory");
        let git = env::split_paths(&env::var_os("PATH").expect("PATH"))
            .map(|directory| directory.join("git"))
            .find(|path| path.is_file())
            .expect("real Git");
        let shim = bin.join("git");
        fs::write(&shim, "#!/bin/sh\nfor argument do\n if [ \"$argument\" = init ]; then\n  printf '%s' \"$$\" > \"$TEST_HELPER\"\n  sleep 60 &\n  printf '%s' \"$!\" > \"$TEST_DESCENDANT\"\n  wait\n fi\ndone\nexec \"$TEST_GIT\" \"$@\"\n")
            .expect("Git shim");
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).expect("executable shim");
        let path = env::join_paths(
            std::iter::once(bin).chain(env::split_paths(&env::var_os("PATH").expect("PATH"))),
        )
        .expect("helper PATH");
        let helper = root.join("helper-pid");
        let descendant = root.join("descendant-pid");
        let child = Command::new(env!("CARGO_BIN_EXE_repot"))
            .args(["new", "cancelled"])
            .current_dir(root)
            .env("HOME", root)
            .env("GHQ_ROOT", root.join("repos"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("GIT_CONFIG_GLOBAL", root.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_CONFIG_COUNT")
            .env_remove("REPOT_CD_FILE")
            .env("PATH", path)
            .env("TEST_GIT", git)
            .env("TEST_HELPER", &helper)
            .env("TEST_DESCENDANT", &descendant)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("repot process");
        let mut running = Running { child, helper };
        let started = Instant::now();
        while !descendant.exists() && started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(descendant.exists(), "Git helper started");
        kill(
            Pid::from_raw(i32::try_from(running.child.id()).expect("PID")),
            signal,
        )
        .expect("send cancellation");
        let cancelled = Instant::now();
        let status = loop {
            if let Some(status) = running.child.try_wait().expect("child state") {
                break status;
            }
            assert!(
                cancelled.elapsed() < Duration::from_secs(5),
                "cancellation deadline"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(
            status.code(),
            Some(1),
            "orderly CLI failure, not signal death"
        );
        assert!(!root.join("repos/local/scratch/cancelled").exists());
        let scratch = root.join("repos/local/scratch");
        assert_eq!(
            fs::read_dir(scratch).expect("scratch parent").count(),
            0,
            "staging unwound"
        );
        let pid = fs::read_to_string(descendant).expect("descendant PID");
        let output = Command::new("ps")
            .args(["-o", "stat=", "-p", pid.trim()])
            .output()
            .expect("inspect helper");
        let state = String::from_utf8_lossy(&output.stdout);
        assert!(
            state.trim().is_empty() || state.trim().starts_with('Z'),
            "helper still live: {state}"
        );
        let mut errors = String::new();
        running
            .child
            .stderr
            .take()
            .expect("stderr")
            .read_to_string(&mut errors)
            .expect("diagnostics");
        assert!(
            errors.contains("operation cancelled"),
            "sanitized cancellation diagnostic: {errors}"
        );
    }

    #[test]
    fn sigterm_unwinds_staging_and_kills_git_helper_group() {
        assert_cancelled(Signal::SIGTERM);
    }

    #[test]
    fn sigint_unwinds_staging_and_kills_git_helper_group() {
        assert_cancelled(Signal::SIGINT);
    }
}
