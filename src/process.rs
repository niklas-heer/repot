//! Bounded subprocesses. Child diagnostics stay private: URLs can contain secrets.

use crate::Result;
use std::ffi::OsStr;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::{Duration, Instant};

static CANCELLATION: OnceLock<Result<Arc<AtomicBool>>> = OnceLock::new();

/// Register once before starting CLI work so subprocesses can unwind on signals.
pub fn install_cancellation() -> Result<()> {
    CANCELLATION
        .get_or_init(|| {
            let cancelled = Arc::new(AtomicBool::new(false));
            #[cfg(unix)]
            for signal in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
                signal_hook::flag::register(signal, Arc::clone(&cancelled))
                    .map_err(|_| "cannot install cancellation handler".to_owned())?;
            }
            Ok(cancelled)
        })
        .as_ref()
        .map(|_| ())
        .map_err(Clone::clone)
}

fn cancellation_requested() -> bool {
    CANCELLATION
        .get()
        .and_then(|result| result.as_ref().ok())
        .is_some_and(|cancelled| cancelled.load(Ordering::Relaxed))
}

pub struct Output {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
}

fn terminate(child: &mut Child) {
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(child.id()) {
        let _ = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGKILL,
        );
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn command(program: &str, args: &[&OsStr], path: &Path) -> Command {
    let mut command = Command::new(program);
    // These variables override `current_dir` and could redirect a bulk operation
    // into a different checkout or index when invoked from a Git-aware shell.
    for name in [
        "GIT_DIR",
        "GIT_COMMON_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_PREFIX",
        "GIT_NAMESPACE",
    ] {
        command.env_remove(name);
    }
    if program == "git" {
        command.args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "maintenance.auto=false",
            "-c",
            "gc.auto=0",
        ]);
    }
    command
        .args(args)
        .current_dir(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("LC_ALL", "C")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_ALLOW_PROTOCOL", "file:https:http:ssh:git:codecommit");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
}

pub fn run(program: &str, args: &[&OsStr], path: &Path, timeout: Duration) -> Result<Output> {
    if cancellation_requested() {
        return Err("operation cancelled".into());
    }
    let mut child = command(program, args, path)
        .spawn()
        .map_err(|error| format!("cannot run {program}: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("cannot capture {program} output"))?;
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut text = String::new();
        // Bound memory even if a broken helper produces unbounded output.
        let result = stdout
            .take(16 * 1024 * 1024 + 1)
            .read_to_string(&mut text)
            .and_then(|_| {
                if text.len() > 16 * 1024 * 1024 {
                    Err(std::io::Error::other("subprocess output limit exceeded"))
                } else {
                    Ok(text)
                }
            });
        let _ = sender.send(result);
    });
    let started = Instant::now();
    let mut status = None;
    let mut output = None;
    loop {
        if cancellation_requested() {
            terminate(&mut child);
            return Err("operation cancelled".into());
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(value) => status = value,
                Err(error) => {
                    terminate(&mut child);
                    return Err(error.to_string());
                }
            }
        }
        if output.is_none() {
            match receiver.try_recv() {
                Ok(value) => output = Some(value),
                Err(mpsc::TryRecvError::Disconnected) => {
                    terminate(&mut child);
                    return Err(format!("cannot read {program} output"));
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let (Some(status), Some(text)) = (status, output.as_ref()) {
            return text
                .as_ref()
                .map(|text| Output {
                    success: status.success(),
                    code: status.code(),
                    stdout: text.clone(),
                })
                .map_err(|_| format!("{program} returned unreadable output"));
        }
        if started.elapsed() >= timeout {
            terminate(&mut child);
            return Err(format!("{program} timed out"));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub fn git(path: &Path, args: &[&str]) -> Result<String> {
    git_optional(path, args)?
        .ok_or_else(|| "Git operation failed; inspect the repository with git".to_owned())
}

pub fn git_optional(path: &Path, args: &[&str]) -> Result<Option<String>> {
    let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    let result = run("git", &args, path, Duration::from_secs(30))?;
    if result.success {
        Ok(Some(
            result
                .stdout
                .strip_suffix('\n')
                .unwrap_or(&result.stdout)
                .to_owned(),
        ))
    } else if result.code == Some(1) {
        Ok(None)
    } else {
        Err("Git operation failed; inspect the repository with git".to_owned())
    }
}
