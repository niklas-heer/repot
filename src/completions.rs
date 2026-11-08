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
        Shell::Nu => return render_nu(&command),
    };
    generator
        .try_generate(&command, &mut io::stdout().lock())
        .map_err(|error| format!("cannot print completions: {error}"))
}

/// Nushell resolves `extern "repot cd"` ahead of the shell-init wrapper of the
/// same name, so the generated definitions for subcommands that hand off a
/// directory are left out; the wrapper then handles them whichever file is
/// sourced first. `extern "repot"` stays, so other subcommands still complete.
fn render_nu(command: &Command) -> Result<()> {
    let mut generated = Vec::new();
    clap_complete_nushell::Nushell
        .try_generate(command, &mut generated)
        .map_err(|error| format!("cannot print completions: {error}"))?;
    let text = String::from_utf8(generated).map_err(|error| error.to_string())?;
    let mut output = io::stdout().lock();
    let mut skipping = false;
    for line in text.split_inclusive('\n') {
        if skipping {
            skipping = line != "  ]\n";
            continue;
        }
        let wrapped = line
            .strip_prefix("  export extern ")
            .and_then(|rest| rest.strip_suffix(" [\n"))
            .map(|name| name.trim_matches('"'))
            .and_then(|name| name.strip_prefix("repot "))
            .is_some_and(|name| crate::navigation::NU_HANDOFF.contains(&name));
        if wrapped {
            skipping = true;
            continue;
        }
        output
            .write_all(line.as_bytes())
            .map_err(|error| format!("cannot print completions: {error}"))?;
    }
    Ok(())
}

pub fn help(path: &[String], mut command: Command) -> Result<()> {
    command.build();
    let mut current = &mut command;
    for name in path {
        current = current
            .find_subcommand_mut(name)
            .ok_or_else(|| format!("unknown command {name:?}"))?;
    }
    let help = current.render_long_help();
    let text = if crate::ui::Paint::stdout().colour() {
        help.ansi().to_string()
    } else {
        help.to_string()
    };
    writeln!(io::stdout().lock(), "{}", text.trim_end())
        .map_err(|error| format!("cannot print help: {error}"))
}
