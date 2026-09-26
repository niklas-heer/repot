//! Apply only revalidated fast-forward and merged-branch return plans.

use crate::Result;
use crate::config::Config;
use crate::status::{self, Options, Report};

pub fn run(config: &Config, options: &Options, dry_run: bool) -> Result<u8> {
    let effective = Options {
        json: options.json,
        jobs: options.jobs,
        timeout: options.timeout,
        no_fetch: options.no_fetch || dry_run,
    };
    if dry_run {
        eprintln!("repot: dry-run uses cached remote refs; no fetch or checkout changes");
    }
    let mut reports = status::collect(config, &effective)?;
    if !dry_run {
        for report in &mut reports {
            if report.plan.is_some() && apply(&effective, report).is_err() {
                report.fail("update refused or failed; repository preserved for manual review");
            }
        }
    }
    status::render(&reports, options.json)
}

fn apply(options: &Options, report: &mut Report) -> Result<()> {
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
