//! Shell completion and help rendering from the authoritative CLI definition.

use std::io::{self, Write};

use clap::{Command, ValueEnum};
use clap_complete::Generator;

use crate::Result;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    Nu,
}

pub fn render(shell: Shell, mut command: Command) -> Result<()> {
    command.set_bin_name("repot");
    command.build();
    let generator: &dyn Generator = match shell {
        Shell::Bash => &clap_complete::shells::Bash,
        Shell::Zsh => &clap_complete::shells::Zsh,
        Shell::Fish => &clap_complete::shells::Fish,
        Shell::Nu => &clap_complete_nushell::Nushell,
    };
    generator
        .try_generate(&command, &mut io::stdout().lock())
        .map_err(|error| format!("cannot print completions: {error}"))
}

pub fn help(path: &[String], mut command: Command) -> Result<()> {
    command.build();
    let mut current = &mut command;
    for name in path {
        current = current
            .find_subcommand_mut(name)
            .ok_or_else(|| format!("unknown command {name:?}"))?;
    }
    writeln!(io::stdout().lock(), "{}", current.render_long_help())
        .map_err(|error| format!("cannot print help: {error}"))
}
