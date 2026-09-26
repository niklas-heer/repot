//! Exact operation-marker microbenchmark; use scripts/benchmark.py for full commands.
#[path = "../src/git_read.rs"]
mod git_read;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Instant;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn git_markers(path: &Path) -> Result<bool> {
    for marker in git_read::OPERATION_MARKERS {
        let output = Command::new("git")
            .current_dir(path)
            .args(["rev-parse", "--git-path", marker])
            .output()?;
        if !output.status.success() {
            return Err("Git layout lookup failed".into());
        }
        let output = String::from_utf8(output.stdout)?;
        if path
            .join(output.strip_suffix('\n').unwrap_or(&output))
            .try_exists()?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn native_markers(path: &Path) -> Result<bool> {
    git_read::Layout::open(path)
        .and_then(|layout| layout.operation_in_progress())
        .ok_or_else(|| "unsupported native layout".into())
}

fn run() -> Result<()> {
    let paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if paths.is_empty() {
        return Err("pass one or more isolated fixture repository paths".into());
    }
    for path in &paths {
        if git_markers(path)? != native_markers(path)? {
            return Err("native/Git result mismatch".into());
        }
    }
    let mut git_ms = Vec::new();
    let mut native_ms = Vec::new();
    for iteration in 0..10 {
        // Alternate ordering; discard the first complete warm-up sample.
        for native in [iteration % 2 == 0, iteration % 2 != 0] {
            let start = Instant::now();
            for path in &paths {
                std::hint::black_box(if native {
                    native_markers(path)?
                } else {
                    git_markers(path)?
                });
            }
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            if iteration != 0 {
                if native {
                    native_ms.push(elapsed);
                } else {
                    git_ms.push(elapsed);
                }
            }
        }
    }
    println!(
        "{}",
        serde_json::json!({
            "repositories": paths.len(), "git_samples_ms": git_ms, "native_samples_ms": native_ms,
            "scope": "Eight operation marker paths and existence checks; identical results",
            "limitations": "Git child startup included; production timeout runner polling excluded"
        })
    );
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
