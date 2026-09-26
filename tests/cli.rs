//! Behaviour of the installed binary, driven as a subprocess.

use std::io;
use std::process::{Command, Output};

fn repot(args: &[&str]) -> io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_repot"))
        .args(args)
        .output()
}

#[test]
fn version_reports_the_package_version() {
    let output = repot(&["--version"]).expect("repot runs");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("repot {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn bare_invocation_prints_usage_and_fails() {
    let output = repot(&[]).expect("repot runs");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Usage: repot"));
}
