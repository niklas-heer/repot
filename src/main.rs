//! repot keeps every Git repository on a machine organised, current and portable.
//!
//! No commands are implemented yet; `BUILD_BRIEF.md` describes the planned scope.

use std::process::ExitCode;

use clap::Parser;

/// Keep every Git repository on this machine organised, current and portable.
#[derive(Debug, Parser)]
#[command(name = "repot", version, about, arg_required_else_help = true)]
struct Cli {}

fn main() -> ExitCode {
    let Cli {} = Cli::parse();
    ExitCode::SUCCESS
}
