//! Safe, portable management of Git checkouts in a ghq-compatible tree.

mod completions;
mod config;
mod discovery;
mod get;
mod ghq_listing;
mod git_read;
mod lifecycle;
mod manifest;
mod manifest_format;
mod mcp;
mod navigation;
mod process;
mod publish;
mod remote_extra;
mod remote_spec;
mod repository_ops;
mod status;
mod sync;

use clap::{Args, CommandFactory, Parser, Subcommand};
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

pub(crate) type Result<T> = std::result::Result<T, String>;

/// Keep every Git repository on this machine organised, current and portable.
#[derive(Debug, Parser)]
#[command(
    name = "repot",
    version,
    about,
    arg_required_else_help = true,
    disable_version_flag = true,
    propagate_version = true,
    disable_help_subcommand = true
)]
struct Cli {
    #[arg(short = 'v', long = "version", visible_short_alias = 'V', global = true, action = clap::ArgAction::Version)]
    _version: Option<bool>,
    /// Manifest location (.kdl selects KDL, .yaml/.yml selects YAML; otherwise TOML).
    #[arg(long, global = true)]
    manifest: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Serve typed repository tools over MCP stdio for coding agents.
    Mcp,
    /// Print the bundled agent guide with automation examples and safety rules.
    AgentGuide,
    /// Print shell completions without changing shell configuration.
    Completions {
        #[arg(value_enum)]
        shell: completions::Shell,
    },
    /// Show help for a command or nested subcommand.
    #[command(visible_alias = "h")]
    Help { command: Vec<String> },
    /// Clone repositories into the tree, or safely update existing checkouts.
    #[command(alias = "clone")]
    Get(get::Options),
    /// Create an empty repository directly at its host/owner/name tree location.
    Create(get::CreateOptions),
    /// Remove a checkout from the active tree into a recoverable archive.
    Rm(repository_ops::RemoveOptions),
    /// Move an existing checkout into the tree and register it.
    Migrate(repository_ops::MigrateOptions),
    /// List or restore repositories removed from the active tree.
    Trash {
        #[command(subcommand)]
        command: repository_ops::TrashCommand,
    },
    /// List checkouts in ghq roots and registered locations.
    List(ghq_listing::ListOptions),
    /// Show the primary repository root, or all roots in priority order.
    Root {
        #[arg(long)]
        all: bool,
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
    match &cli.command {
        Commands::Mcp => return mcp::run(cli.manifest.clone()),
        Commands::AgentGuide => {
            std::io::stdout()
                .lock()
                .write_all(include_bytes!("../docs/agents.md"))
                .map_err(|error| format!("cannot print agent guide: {error}"))?;
            return Ok(0);
        }
        Commands::ShellInit { shell } => {
            navigation::shell_init(shell)?;
            return Ok(0);
        }
        Commands::Completions { shell } => {
            completions::render(*shell, Cli::command())?;
            return Ok(0);
        }
        Commands::Help { command } => {
            completions::help(command, Cli::command())?;
            return Ok(0);
        }
        _ => {}
    }
    process::install_cancellation()?;
    let config = if matches!(cli.command, Commands::Adopt { .. } | Commands::Migrate(_)) {
        config::Config::load_for_write(cli.manifest.as_deref())?
    } else {
        config::Config::load(cli.manifest.as_deref())?
    };
    match cli.command {
        Commands::Get(options) => return get::run(&config, &options),
        Commands::Create(options) => return get::create(&config, &options),
        Commands::Rm(options) => return repository_ops::remove(&config, &options),
        Commands::Migrate(options) => return repository_ops::migrate(&config, &options),
        Commands::Trash { command } => return repository_ops::trash(&config, &command),
        Commands::List(options) => return ghq_listing::list(&config, &options),
        Commands::Root { all } => return ghq_listing::root(&config, all),
        Commands::Jump { query } => {
            navigation::jump(&discovery::discover(&config)?, query.as_deref())?;
        }
        Commands::Mcp
        | Commands::AgentGuide
        | Commands::ShellInit { .. }
        | Commands::Completions { .. }
        | Commands::Help { .. } => {}
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
