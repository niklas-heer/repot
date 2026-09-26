+++
schema_version = 1
id = "01M3FPW5SDKZT76CXJVKKB17E1"
title = "Create scratch projects and relocate atomically"
date = "2026-09-26"
status = "accepted"
tags = ["safety", "architecture"]
supersedes = []
superseded_by = []
depends_on = []
related_to = []
+++
## Decision

Create scratch projects under the primary ghq root at `local/<namespace>/<name>`, with `scratch` as the default namespace. Relocate standalone checkouts using the operating system's atomic no-overwrite rename. Refuse cross-filesystem moves, linked worktrees, submodules and borrowed object databases; offer registration in place.

## Why

This follows the prior scratch convention and the delegated build brief. Copy-and-delete recovery would risk partly moved work. Rustix exposes Linux and macOS no-overwrite rename safely without application unsafe code and is already a transitive dependency.

## Consequences

Existing paths, including dangling symlinks and concurrently created directories, cannot be overwritten. Creation stages Git initialization privately before making the checkout visible. Moving retains uncommitted files and branches. Some layouts require manual relocation or registration rather than automatic repair.
