+++
schema_version = 1
id = "01M3HEXBH6JVZTDSYGHXE6PV3N"
title = "Report checkout drift with doctor and fix commands"
date = "2026-09-27"
status = "accepted"
tags = ["cli", "ux"]
supersedes = []
superseded_by = []
depends_on = []
related_to = []
+++
## Decision

Add `repot doctor`, a report-only command that finds checkouts which drifted
out of place and prints the command that fixes each: duplicates of the same
repository, repositories renamed, transferred, archived or missing on GitHub,
and checkouts whose folder no longer matches `host/owner/name`. Identity comes
from the checkout's `origin` (or first) remote. When `gh` is installed and
`--offline` is not given, GitHub is asked through `gh api repos/OWNER/NAME`,
which follows renames and transfers; any other failure says nothing. Duplicates
are grouped by GitHub's current name, keeping the copy already at that name's
tree location. Before suggesting `repot rm`, doctor lists work that exists only in
the stale copy.

## Context

On 2026-09-27 the user asked for a way to detect and clean up repositories left
behind after renames. Neither the remote URL nor the path of an old checkout
changes when a repository is renamed, and Git keeps fetching through GitHub's
redirect, so only the forge knows the new name. Comparing root commits was
rejected: forks share history and would be reported as duplicates.

## Consequences

Doctor never moves, removes or reconfigures anything; users run the suggested
`repot rm`, `git remote set-url` and `repot adopt` commands, which keep their own
safety checks and dry runs. It needs `gh` and network access to see renames;
other forges are checked offline only. One GitHub API call per GitHub checkout
runs at most eight at a time, well within API limits for a personal machine.

## Update 2026-09-27

At the user's request, `--fix` now applies the suggested commands after asking
per finding, and `--fix --yes` applies them unattended while skipping every
finding whose checkout holds work that exists nowhere else. Fixes call the
existing `repot rm` and `repot adopt` paths, so archive recovery, move checks and
manifest updates are unchanged. The report alone still changes nothing.

