+++
schema_version = 1
id = "01M3FP7EBB02S16Q0KFHT100AF"
title = "Separate safe sync plans from checkout mutations"
date = "2026-09-26"
status = "accepted"
tags = ["safety", "git"]
supersedes = []
superseded_by = []
depends_on = []
related_to = []
+++
## Decision

Keep sync planning separate from execution. Fetch branches into remote-tracking refs with bounded parallelism, then classify each checkout. Recheck safety immediately before fast-forwarding or switching. A dry-run uses cached refs and never fetches; output explicitly identifies that limitation.

## Why

The delegated build brief requires preserving work, fast-forward-only updates and reproducible dry-runs. Fetch mutates refs, so a literally nonmutating dry-run cannot promise remote freshness. Remote default ancestry, pruned-upstream patch equivalence or a merged GitHub PR with the exact head and default base provide branch-return evidence.

## Consequences

Missing or ambiguous proof requires review. Linked worktrees can be inspected but target branches checked out elsewhere are not switched into. Submodules and in-progress Git operations require manual handling. Exit codes distinguish completion (0), operational failure (1), usage (2) and review (3). GitHub PR evidence uses the optional authenticated gh CLI.
