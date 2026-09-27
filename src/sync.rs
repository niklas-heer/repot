//! Apply only revalidated fast-forward and merged-branch return plans.

use crate::Result;
use crate::config::Config;
use crate::status::{self, Options, Report};
use crate::ui;

pub fn run(config: &Config, options: &Options, dry_run: bool) -> Result<u8> {
    let started = std::time::Instant::now();
    let effective = Options {
        json: options.json,
        jobs: options.jobs,
        timeout: options.timeout,
        no_fetch: options.no_fetch || dry_run,
    };
    if dry_run {
        ui::note(
            "·",
            ui::DIM,
            "dry run: planning from cached remote refs; nothing is fetched or changed",
        );
    }
    let update = |report: &mut Report| {
        // Each repository is revalidated and updated independently, right after
        // its own inspection; no two workers ever touch the same checkout.
        if report.plan.is_some() && apply(&effective, report).is_err() {
            report.fail("update refused or failed; repository preserved for manual review");
        }
    };
    let finish: status::Finish<'_> = &update;
    let reports = {
        let progress = (!options.json).then(|| ui::Progress::start("Syncing", 0));
        status::collect(
            config,
            &effective,
            progress.as_ref(),
            (!dry_run).then_some(finish),
        )?
    };
    status::render(
        &reports,
        options.json,
        &status::View {
            roots: &config.roots,
            mode: if dry_run {
                status::Mode::SyncPlan
            } else {
                status::Mode::Sync
            },
            elapsed: started.elapsed(),
        },
    )
}

pub fn apply(options: &Options, report: &mut Report) -> Result<()> {
    status::validate_plan(report, options)?;
    let plan = report.plan.as_ref().ok_or("missing update plan")?;
    if let Some(branch) = &plan.branch {
        let mut args = vec![
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "submodule.recurse=false",
            "switch",
            "--no-guess",
            "--no-overwrite-ignore",
        ];
        if plan.previous_default.is_none() {
            args.extend(["--create", branch, &plan.target]);
        } else {
            args.push(branch);
        }
        if status::probe(&report.path, &args, options.timeout)?.is_none() {
            return Err("branch switch refused".into());
        }
        if plan.previous_default.is_none()
            && status::probe(
                &report.path,
                &["branch", "--set-upstream-to", &plan.reference, branch],
                options.timeout,
            )?
            .is_none()
        {
            return Err("cannot configure default branch tracking".into());
        }
    }
    let result = status::probe(
        &report.path,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "submodule.recurse=false",
            "merge",
            "--ff-only",
            "--no-edit",
            "--no-autostash",
            "--no-overwrite-ignore",
            &plan.target,
        ],
        options.timeout,
    )?;
    if result.is_none() {
        return Err("fast-forward refused".into());
    }
    report.applied = true;
    Ok(())
}
