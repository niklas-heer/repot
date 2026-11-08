+++
schema_version = 1
id = "01M6WGFARZSW7QDQW193X3ZJ5F"
title = "Read checkout state with one porcelain call and the cached remote HEAD natively"
date = "2026-11-08"
status = "proposed"
tags = ["performance", "architecture"]
supersedes = []
superseded_by = []
depends_on = []
related_to = ["01M3FTDH95X899C1P70XP7RA91"]
+++
## Decision

Inspect each checkout for `status` and `sync` with one `git status --porcelain=v2 --branch --show-stash -z` call instead of separate branch, upstream, ahead/behind and stash reads. Read the cached `refs/remotes/<remote>/HEAD` symbolic ref through gix, falling back to `git symbolic-ref` whenever native layout recognition declines. Every pre-mutation recheck in `validate_plan` keeps its own Git CLI commands.

## Context

Council review on 2026-11-08 counted 14 Git processes per clean checkout for cached status (2,800 for 200 checkouts), which made `status` and `sync` take seconds without any network. Consolidating the reads brings a clean checkout to 4 processes: cached status over 100 clones dropped from 2.47 s to 0.76 s with `--jobs 8`, and from 6.48 s to 1.62 s with `--jobs 1` (see docs/benchmarks). A differential run over 35 synthetic checkout states produced byte-identical cached status, fetching status, `sync --dry-run` and `sync --json` output before and after.

The cached remote HEAD only chooses which branch inspection treats as the default. A return still needs Git's ancestry or patch-identity proof, and `validate_plan` rechecks every local precondition with Git before changing anything. A differential test compares the native read with `git symbolic-ref`, including packed refs. Keeping the read on Git would cost one more process per checkout.

## Consequences

This narrows the earlier rule that Git keeps authority over refs: one cached symbolic ref used for inspection is read natively. Mutations and pre-mutation checks stay on the Git CLI. Reftable repositories, which gix may not recognise, fall back to Git. The per-checkout Git process budget is enforced by tests/status_spawns.rs.
