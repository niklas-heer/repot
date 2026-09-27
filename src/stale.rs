//! Repositories you have not touched in a long time: candidates to archive with
//! `repot rm`, which keeps them recoverable.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::config::Config;
use crate::visits::{self, Visits};
use crate::{Result, discovery, doctor, ui, work};

#[derive(Debug, clap::Args)]
pub struct Options {
    /// Days without a visit or Git activity of your own.
    #[arg(long, default_value_t = 90, value_parser = clap::value_parser!(u64).range(1..=36500))]
    pub days: u64,
    /// Print machine-readable results.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Serialize)]
pub struct Stale {
    pub path: PathBuf,
    /// Unix time of the last visit or own Git activity, if any is known.
    pub last_touched: Option<u64>,
    /// Work that exists only in this checkout.
    pub local_work: Option<String>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// The later of your last visit and your last own Git activity.
pub fn last_touched(path: &Path, visits: &HashMap<PathBuf, Visits>) -> Option<u64> {
    let visited = visits.get(path).and_then(|visits| visits.last);
    visited.max(visits::activity(path))
}

fn candidates(config: &Config, days: u64) -> Result<Vec<(PathBuf, Option<u64>)>> {
    let visits = visits::load();
    let cutoff = now().saturating_sub(days.saturating_mul(86_400));
    Ok(discovery::discover(config)?
        .into_iter()
        .map(|repository| {
            let touched = last_touched(&repository.path, &visits);
            (repository.path, touched)
        })
        .filter(|(_, touched)| touched.is_none_or(|touched| touched < cutoff))
        .collect())
}

pub fn run(config: &Config, options: &Options) -> Result<u8> {
    let mut candidates = candidates(config, options.days)?;
    // Oldest first: the most forgotten checkouts lead the list.
    candidates.sort_by_key(|(path, touched)| (*touched, path.clone()));
    let stale: Vec<Stale> = work::parallel(&candidates, 16, None, |(path, touched)| Stale {
        path: path.clone(),
        last_touched: *touched,
        local_work: doctor::local_work(path),
    })?;
    if options.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&stale).map_err(|error| error.to_string())?
        );
        return Ok(0);
    }
    let paint = ui::Paint::stdout();
    let mut out = String::from("\n");
    if stale.is_empty() {
        let _ = writeln!(
            out,
            " {} {}",
            paint.paint(ui::GOOD.bold(), "✓"),
            paint.paint(
                ui::BOLD,
                format!(
                    "Every repository was touched in the last {} days",
                    options.days
                )
            )
        );
        print!("{out}");
        return Ok(0);
    }
    let _ = writeln!(
        out,
        " {} {}  {}  {}",
        paint.paint(ui::INFO.bold(), "◌"),
        paint.paint(ui::BOLD, format!("Untouched for {}+ days", options.days)),
        paint.paint(ui::DIM, stale.len()),
        paint.paint(ui::DIM, "· candidates to archive")
    );
    let width = stale
        .iter()
        .map(|entry| ui::width(&ui::repository_name(&entry.path, &config.roots)))
        .max()
        .unwrap_or_default()
        .min(ui::terminal_width() / 2);
    for entry in &stale {
        let name = ui::repository_name(&entry.path, &config.roots);
        let when = entry
            .last_touched
            .map_or_else(|| "never".to_owned(), visits::ago);
        let mut line = format!(
            "   {}  {}",
            paint.paint(ui::BOLD, ui::pad(&ui::truncate(&name, width), width)),
            paint.paint(ui::DIM, format!("{when:<12}"))
        );
        if let Some(work) = &entry.local_work {
            let _ = write!(
                line,
                "{}",
                paint.paint(ui::WARN, format!("only here: {work}"))
            );
        }
        let _ = writeln!(out, "{}", line.trim_end());
    }
    let _ = writeln!(
        out,
        "\n {} archive one with {}; {} brings it back",
        paint.paint(ui::INFO, "→"),
        paint.paint(ui::BOLD, "repot rm NAME"),
        paint.paint(ui::BOLD, "repot trash restore")
    );
    print!("{out}");
    Ok(0)
}

/// At most once a week, a one-line reminder after `repot status` when
/// repositories have gone untouched for 90 days.
pub fn weekly_hint(config: &Config) -> Option<String> {
    let marker = visits::log_path()?.with_file_name("stale-hint");
    let shown_recently = fs::metadata(&marker)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age < Duration::from_hours(7 * 24));
    if shown_recently {
        return None;
    }
    let count = candidates(config, 90).ok()?.len();
    if count == 0 {
        return None;
    }
    fs::create_dir_all(marker.parent()?).ok()?;
    fs::write(&marker, b"").ok()?;
    let paint = ui::Paint::stdout();
    Some(format!(
        " {} {count} {} untouched for 90+ days; see {}\n",
        paint.paint(ui::INFO, "→"),
        if count == 1 {
            "repository"
        } else {
            "repositories"
        },
        paint.paint(ui::BOLD, "repot stale")
    ))
}
