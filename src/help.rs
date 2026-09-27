//! The grouped overview behind `repot --help`, styling for every command's
//! help, and the small logo.

use std::fmt::Write as _;

use clap::Command;
use clap::builder::StyledStr;
use clap::builder::styling::{AnsiColor, Styles};

use crate::ui::{BOLD, DIM, LEAF, POT, Style};

const LOGO: [(&str, &str); 6] = [
    ("      ○", ""),
    ("   ○╮ │ ╭○", ""),
    ("    ╰─┼─╯", ""),
    ("", "  ▟███████▙"),
    ("", "   ▜█████▛"),
    ("", "    ▀▀▀▀▀"),
];

/// Commands as a newcomer meets them: getting around, staying current,
/// bringing repositories in, cleaning up, and one-time setup.
const GROUPS: &[(&str, &[(&str, &str)])] = &[
    (
        "Get around",
        &[
            (
                "cd [query]",
                "Jump to a repository; opens an inline fuzzy picker",
            ),
            (
                "info [query]",
                "Show branch, changes, remote and recent commits",
            ),
            ("list [query]", "List repositories"),
            ("root", "Show where repositories live"),
        ],
    ),
    (
        "Stay current",
        &[
            ("status", "Fetch everything and show what needs attention"),
            (
                "sync",
                "Fast-forward what is safe; return merged branches to main",
            ),
        ],
    ),
    (
        "Bring repositories in",
        &[
            (
                "clone <repo>…",
                "Clone into the tree, e.g. owner/name (alias: get)",
            ),
            (
                "new <name>",
                "Start a local project; owner/name creates it in the tree",
            ),
            (
                "publish <owner/name>",
                "Create the remote, push, and move into the tree",
            ),
            ("scan [path]", "Find repositories outside the tree"),
            (
                "adopt <path>",
                "Move a stray into the tree, or --register it in place",
            ),
            (
                "restore",
                "Clone every missing repository from your manifest",
            ),
        ],
    ),
    (
        "Clean up",
        &[
            ("rm <repo>", "Archive a checkout; nothing is deleted"),
            ("trash", "List or restore archived checkouts"),
            (
                "doctor",
                "Find duplicates, renamed repositories and misplaced checkouts",
            ),
        ],
    ),
    (
        "Set up",
        &[
            ("shell-init <shell>", "Let `repot cd` change your directory"),
            ("completions <shell>", "Print tab completions"),
            ("agent-guide", "Print the guide for coding agents"),
            ("mcp", "Serve repot to agents over MCP"),
        ],
    ),
];

const OPTIONS: &[(&str, &str)] = &[
    (
        "--manifest <path>",
        "Use this manifest instead of ~/.config/repot/repos.*",
    ),
    (
        "-h, --help",
        "Show help; `repot help <command>` explains one command",
    ),
    ("-v, --version", "Show the version"),
];

fn styled(style: Style, text: &str) -> String {
    if text.is_empty() {
        String::new()
    } else {
        format!("{style}{text}{style:#}")
    }
}

/// The overview printed by `repot`, `repot -h` and `repot help`.
pub fn overview() -> String {
    let heading = GREEN_BOLD;
    let literal = CYAN_BOLD;
    let mut text = String::new();
    let about = [
        styled(BOLD, &format!("repot {}", env!("CARGO_PKG_VERSION"))),
        "Keep every Git repository on this machine".to_owned(),
        "organised, current and portable.".to_owned(),
        String::new(),
        styled(DIM, "Updates are fast-forward only and nothing is"),
        styled(DIM, "ever committed, stashed, reset or deleted."),
    ];
    for ((plant, pot), about) in LOGO.iter().zip(about) {
        let art = format!("{}{}", styled(LEAF, plant), styled(POT, pot));
        let padding =
            " ".repeat(12_usize.saturating_sub(plant.chars().count().max(pot.chars().count())));
        let _ = writeln!(text, "{}", format!("{art}{padding}  {about}").trim_end());
    }
    let _ = writeln!(
        text,
        "\n{} {} <command> [options]",
        styled(heading, "Usage:"),
        styled(literal, "repot"),
    );
    let width = GROUPS
        .iter()
        .flat_map(|(_, commands)| commands.iter())
        .map(|(usage, _)| usage.chars().count())
        .chain(OPTIONS.iter().map(|(usage, _)| usage.chars().count()))
        .max()
        .unwrap_or_default();
    let row = |usage: &str, about: &str| {
        let (name, arguments) = usage.split_once(' ').unwrap_or((usage, ""));
        let padding = " ".repeat(width.saturating_sub(usage.chars().count()));
        let separator = if arguments.is_empty() { "" } else { " " };
        format!(
            "  {}{separator}{arguments}{padding}  {about}\n",
            styled(literal, name)
        )
    };
    for (group, commands) in GROUPS {
        let _ = write!(text, "\n{}\n", styled(heading, group));
        for (usage, about) in *commands {
            text.push_str(&row(usage, about));
        }
    }
    let _ = write!(text, "\n{}\n", styled(heading, "Options"));
    for (usage, about) in OPTIONS {
        let padding = " ".repeat(width.saturating_sub(usage.chars().count()));
        let _ = writeln!(text, "  {}{padding}  {about}", styled(literal, usage));
    }
    let _ = writeln!(
        text,
        "\n{}",
        styled(
            DIM,
            "Every command that changes something accepts --dry-run. Scripts can use --json."
        )
    );
    text
}

const GREEN_BOLD: Style = Style::new()
    .bold()
    .fg_color(Some(clap::builder::styling::Color::Ansi(AnsiColor::Green)));
const CYAN_BOLD: Style = Style::new()
    .bold()
    .fg_color(Some(clap::builder::styling::Color::Ansi(AnsiColor::Cyan)));

/// Apply repot's help styling to the whole command tree and install the overview.
pub fn command(command: Command) -> Command {
    command
        .styles(
            Styles::styled()
                .header(GREEN_BOLD)
                .usage(GREEN_BOLD)
                .literal(CYAN_BOLD)
                .placeholder(
                    Style::new()
                        .fg_color(Some(clap::builder::styling::Color::Ansi(AnsiColor::Cyan))),
                )
                .error(
                    Style::new()
                        .bold()
                        .fg_color(Some(clap::builder::styling::Color::Ansi(AnsiColor::Red))),
                )
                .valid(GREEN_BOLD)
                .invalid(
                    Style::new()
                        .bold()
                        .fg_color(Some(clap::builder::styling::Color::Ansi(AnsiColor::Yellow))),
                ),
        )
        .override_help(StyledStr::from(overview()))
}

/// Every visible subcommand, so tests can prove the overview stays complete.
#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory as _;

    #[test]
    fn overview_lists_every_visible_command() {
        let overview = overview();
        for subcommand in crate::Cli::command().get_subcommands() {
            if subcommand.is_hide_set() || subcommand.get_name() == "help" {
                continue;
            }
            assert!(
                overview.contains(&format!("{}\u{1b}[0m", subcommand.get_name()))
                    || overview.contains(&format!("{} ", subcommand.get_name())),
                "{} missing from the overview",
                subcommand.get_name()
            );
        }
    }
}
