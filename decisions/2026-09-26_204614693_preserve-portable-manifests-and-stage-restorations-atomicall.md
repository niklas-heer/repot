+++
schema_version = 1
id = "01M3FQFW55W1EVXPSMJ2ZZHMX7"
title = "Preserve portable manifests and stage restorations atomically"
date = "2026-09-26"
status = "accepted"
tags = ["architecture", "safety"]
supersedes = []
superseded_by = []
depends_on = []
related_to = []
+++
## Decision

Keep explicit repository registrations in the TOML manifest. Use toml_edit to preserve comments, follow manifest symlinks to their dotfiles targets, and serialize writes using a persistent companion file lock and reread. Adopt can move dirty standalone checkouts without altering their contents or register them in place. Restore stages each clone and installs it with atomic no-overwrite rename.

## Why

The delegated brief centers portability and preserving work. Human-maintained dotfiles should retain comments and symlinks. Concurrent coding agents must not silently replace each other's registrations. A partial clone must not occupy a final destination and look like a restored checkout.

## Consequences

Existing destinations and restore=false entries are untouched. Home-relative paths remain portable. Failures are isolated per clone; a failed manifest save after a move reports its actual location without destructive rollback. Cross-filesystem and dependent Git layouts require registration or manual relocation.
