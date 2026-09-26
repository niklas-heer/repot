//! Safe, portable management of Git checkouts in a ghq-compatible tree.

mod config;
mod discovery;
mod lifecycle;
mod manifest;
mod navigation;
mod process;
mod publish;
mod status;
mod sync;

use clap::{Args, Parser, Subcommand};
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
    /// Create a scratch checkout without a remote or commit.
    New {
        name: String,
        #[arg(long, default_value = "scratch")]
        namespace: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Create a forge repository, push the current branch, and move the checkout.
    Publish(publish::Options),
    /// Search for unregistered checkouts outside ghq roots.
    Find {
        /// Search directory (defaults to HOME).
        path: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Move a stray checkout into the tree, or register it in place.
    Adopt {
        path: PathBuf,
        #[arg(long)]
        register: bool,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
    /// Clone missing manifest entries, leaving every existing path untouched.
    Restore {
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u64).range(1..=3600))]
        timeout: u64,
    },
    /// Fetch and report repository states and recommended actions.
    Status {
        #[command(flatten)]
        inspection: Inspection,
        /// Inspect cached refs without fetching or changing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Fast-forward safe checkouts and return proven merged branches.
    Sync {
        #[command(flatten)]
        inspection: Inspection,
        /// Plan against cached remote refs without changing anything.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Debug, Args)]
struct Inspection {
    #[arg(long)]
    json: bool,
    /// Maximum concurrent repositories.
    #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u8).range(1..=32))]
    jobs: u8,
    /// Network subprocess timeout in seconds.
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=3600))]
    timeout: u64,
    /// Inspect cached remote refs without fetching.
    #[arg(long)]
    no_fetch: bool,
}

impl Inspection {
    fn options(&self) -> status::Options {
        status::Options {
            json: self.json,
            jobs: usize::from(self.jobs),
            timeout: std::time::Duration::from_secs(self.timeout),
            no_fetch: self.no_fetch,
        }
    }
}

fn run(cli: Cli) -> Result<u8> {
    if let Commands::ShellInit { shell } = &cli.command {
        navigation::shell_init(shell)?;
        return Ok(0);
    }
    let config = if matches!(cli.command, Commands::Adopt { .. }) {
        config::Config::load_for_write(cli.manifest.as_deref())?
    } else {
        config::Config::load(cli.manifest.as_deref())?
    };
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
        Commands::New {
            name,
            namespace,
            dry_run,
        } => {
            lifecycle::new_project(&config, &name, &namespace, dry_run)?;
        }
        Commands::Publish(options) => return publish::run(&config, &options),
        Commands::Find { path, json } => {
            let path = path
                .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
                .ok_or("HOME must be set or supply a search path")?;
            return manifest::find(&config, &path, json);
        }
        Commands::Adopt {
            path,
            register,
            dry_run,
            json,
        } => {
            return manifest::adopt(&config, &path, register, dry_run, json);
        }
        Commands::Restore {
            dry_run,
            json,
            timeout,
        } => {
            return manifest::restore(
                &config,
                dry_run,
                json,
                std::time::Duration::from_secs(timeout),
            );
        }
        Commands::Status {
            inspection,
            dry_run,
        } => {
            let mut options = inspection.options();
            options.no_fetch |= dry_run;
            if dry_run {
                eprintln!("repot: dry-run uses cached remote refs; no fetch or checkout changes");
            }
            return status::run(&config, &options);
        }
        Commands::Sync {
            inspection,
            dry_run,
        } => {
            return sync::run(&config, &inspection.options(), dry_run);
        }
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
