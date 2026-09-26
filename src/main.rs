//! Safe, portable management of Git checkouts in a ghq-compatible tree.

mod config;
mod discovery;
mod navigation;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

pub(crate) type Result<T> = std::result::Result<T, String>;

/// Keep every Git repository on this machine organised, current and portable.
#[derive(Debug, Parser)]
#[command(name = "repot", version, about, arg_required_else_help = true)]
struct Cli {
    /// Manifest location (default: `$XDG_CONFIG_HOME/repot/repos.toml`).
    #[arg(long, global = true)]
    manifest: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// List checkouts in ghq roots and registered locations.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Select a repository; install shell-init to change your shell directory.
    Jump { query: Option<String> },
    /// Print shell integration for nu, zsh, bash or fish.
    ShellInit { shell: String },
}

fn run(cli: Cli) -> Result<u8> {
    if let Commands::ShellInit { shell } = &cli.command {
        navigation::shell_init(shell)?;
        return Ok(0);
    }
    let config = config::Config::load(cli.manifest.as_deref())?;
    match cli.command {
        Commands::List { json } => {
            let repositories = discovery::discover(&config)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&repositories)
                        .map_err(|error| error.to_string())?
                );
            } else {
                for repository in repositories {
                    println!("{}", repository.path.display());
                }
            }
        }
        Commands::Jump { query } => {
            navigation::jump(&discovery::discover(&config)?, query.as_deref())?;
        }
        Commands::ShellInit { .. } => {}
    }
    Ok(0)
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("repot: {error}");
            ExitCode::FAILURE
        }
    }
}
