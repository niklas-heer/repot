//! Safe, portable management of Git checkouts in a ghq-compatible tree.

mod completions;
mod config;
mod discovery;
mod get;
mod ghq_listing;
mod git_read;
mod help;
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
mod ui;

use clap::{Args, CommandFactory, FromArgMatches as _, Parser, Subcommand};
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
    /// Print the version.
    #[arg(short = 'v', long = "version", visible_short_alias = 'V', global = true, action = clap::ArgAction::Version, help_heading = "Global options")]
    _version: Option<bool>,
    /// Manifest location (.kdl selects KDL, .yaml/.yml selects YAML; otherwise TOML).
    #[arg(
        long,
        global = true,
        value_name = "PATH",
        help_heading = "Global options"
    )]
    manifest: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

// The order here is the order of `repot help`; see help.rs for the grouped overview.
#[derive(Debug, Subcommand)]
enum Commands {
    /// Jump to a repository: a unique match directly, otherwise an inline fuzzy picker.
    #[command(alias = "jump")]
    Cd {
        /// Part of a repository name; the picker opens with it when several match.
        query: Option<String>,
    },
    /// List repositories in your roots and registered locations.
    #[command(alias = "ls")]
    List(ghq_listing::ListOptions),
    /// Show the primary repository root, or all roots in priority order.
    Root {
        /// Show every root, not just the primary one.
        #[arg(long)]
        all: bool,
    },
    /// Fetch every repository and show what needs attention.
    Status {
        #[command(flatten)]
        inspection: Inspection,
        /// Inspect cached refs without fetching or changing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Fast-forward safe checkouts and return merged branches to the default branch.
    Sync {
        #[command(flatten)]
        inspection: Inspection,
        /// Plan against cached remote refs without changing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Clone repositories into the tree, or safely update existing checkouts.
    #[command(alias = "get")]
    Clone(get::Options),
    /// Start a local project, or an empty repository at its owner/name tree location.
    New {
        /// A plain name starts a scratch project; owner/name, host/owner/name or a URL
        /// creates an empty repository where a clone of it would live.
        name: String,
        /// Folder under <root>/local/ for scratch projects.
        #[arg(long, default_value = "scratch")]
        namespace: String,
        /// Create a bare repository (owner/name targets only).
        #[arg(long)]
        bare: bool,
        /// Show where the project would go without creating it.
        #[arg(long)]
        dry_run: bool,
    },
    /// Create a forge repository, push the current branch, and move the checkout into the tree.
    Publish(publish::Options),
    /// Find repositories outside your roots that repot does not know yet.
    #[command(alias = "find")]
    Scan {
        /// Directory to search (defaults to your home directory).
        path: Option<PathBuf>,
        /// Print machine-readable results.
        #[arg(long)]
        json: bool,
    },
    /// Move a stray checkout into the tree, or register it where it is.
    Adopt {
        /// Checkout to adopt.
        path: PathBuf,
        /// Keep the checkout in place and record it in the manifest.
        #[arg(long)]
        register: bool,
        /// Show what would happen without moving or registering anything.
        #[arg(long)]
        dry_run: bool,
        /// Print machine-readable results.
        #[arg(long)]
        json: bool,
    },
    /// Clone every manifest repository that is missing; existing paths stay untouched.
    Restore {
        /// Show what would be cloned without cloning.
        #[arg(long)]
        dry_run: bool,
        /// Print machine-readable results.
        #[arg(long)]
        json: bool,
        /// Maximum duration of each clone, in seconds.
        #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u64).range(1..=3600))]
        timeout: u64,
    },
    /// Archive a checkout out of the tree; `repot trash` brings it back.
    Rm(repository_ops::RemoveOptions),
    /// List or restore checkouts archived by `repot rm`.
    Trash {
        #[command(subcommand)]
        command: repository_ops::TrashCommand,
    },
    /// Print the shell integration that lets `repot cd` change your directory.
    ShellInit {
        /// Shell to integrate with: nu, zsh, bash or fish.
        shell: String,
    },
    /// Print shell completions without changing shell configuration.
    Completions {
        #[arg(value_enum)]
        shell: completions::Shell,
    },
    /// Print the bundled agent guide with automation examples and safety rules.
    AgentGuide,
    /// Serve typed repository tools over MCP stdio for coding agents.
    Mcp,
    /// Show help for a command or nested subcommand.
    #[command(visible_alias = "h")]
    Help {
        /// Command to explain, for example `trash restore`.
        command: Vec<String>,
    },
    /// Create an empty repository at its tree location (ghq compatibility; prefer `new`).
    #[command(hide = true)]
    Create(get::CreateOptions),
    /// Move an existing checkout into the tree (ghq compatibility; prefer `adopt`).
    #[command(hide = true)]
    Migrate(repository_ops::MigrateOptions),
}

#[derive(Debug, Args)]
struct Inspection {
    /// Print machine-readable results.
    #[arg(long)]
    json: bool,
    /// Maximum concurrent repositories.
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u8).range(1..=32))]
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

/// Commands that must work even when the manifest or roots are broken.
fn run_standalone(cli: &Cli) -> Option<Result<u8>> {
    let result = match &cli.command {
        Commands::Mcp => return Some(mcp::run(cli.manifest.clone())),
        Commands::AgentGuide => std::io::stdout()
            .lock()
            .write_all(include_bytes!("../docs/agents.md"))
            .map_err(|error| format!("cannot print agent guide: {error}")),
        Commands::ShellInit { shell } => navigation::shell_init(shell),
        Commands::Completions { shell } => {
            completions::render(*shell, help::command(Cli::command()))
        }
        Commands::Help { command } => completions::help(command, help::command(Cli::command())),
        _ => return None,
    };
    Some(result.map(|()| 0))
}

fn run(cli: Cli) -> Result<u8> {
    if let Some(result) = run_standalone(&cli) {
        return result;
    }
    process::install_cancellation()?;
    let config = if matches!(cli.command, Commands::Adopt { .. } | Commands::Migrate(_)) {
        config::Config::load_for_write(cli.manifest.as_deref())?
    } else {
        config::Config::load(cli.manifest.as_deref())?
    };
    match cli.command {
        Commands::Cd { query } => {
            navigation::jump(
                &discovery::discover(&config)?,
                &config.roots,
                query.as_deref(),
            )?;
        }
        Commands::List(options) => return ghq_listing::list(&config, &options),
        Commands::Root { all } => return ghq_listing::root(&config, all),
        Commands::Status {
            inspection,
            dry_run,
        } => {
            let mut options = inspection.options();
            options.no_fetch |= dry_run;
            if dry_run {
                ui::note(
                    "·",
                    ui::DIM,
                    "dry run: inspecting cached remote refs; nothing is fetched or changed",
                );
            }
            return status::run(&config, &options);
        }
        Commands::Sync {
            inspection,
            dry_run,
        } => {
            return sync::run(&config, &inspection.options(), dry_run);
        }
        Commands::Clone(options) => return get::run(&config, &options),
        Commands::New {
            name,
            namespace,
            bare,
            dry_run,
        } => return start(&config, name, &namespace, bare, dry_run),
        Commands::Publish(options) => return publish::run(&config, &options),
        Commands::Scan { path, json } => {
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
        Commands::Rm(options) => return repository_ops::remove(&config, &options),
        Commands::Trash { command } => return repository_ops::trash(&config, &command),
        Commands::Create(options) => return get::create(&config, &options),
        Commands::Migrate(options) => return repository_ops::migrate(&config, &options),
        Commands::Mcp
        | Commands::AgentGuide
        | Commands::ShellInit { .. }
        | Commands::Completions { .. }
        | Commands::Help { .. } => {}
    }
    Ok(0)
}

/// A path-like name means "where a clone of this would live"; a plain name is
/// a scratch project. One verb covers both ways to start something.
fn start(
    config: &config::Config,
    name: String,
    namespace: &str,
    bare: bool,
    dry_run: bool,
) -> Result<u8> {
    if name.contains(['/', ':']) {
        return get::create(
            config,
            &get::CreateOptions {
                repository: name,
                vcs: None,
                bare,
                dry_run,
            },
        );
    }
    if bare {
        return Err(
            "--bare needs an owner/name location; scratch projects always have a working tree"
                .into(),
        );
    }
    lifecycle::new_project(config, &name, namespace, dry_run)?;
    Ok(0)
}

fn main() -> ExitCode {
    let matches = help::command(Cli::command()).get_matches();
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(error) => error.exit(),
    };
    match run(cli) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            ui::error(&error);
            ExitCode::FAILURE
        }
    }
}
