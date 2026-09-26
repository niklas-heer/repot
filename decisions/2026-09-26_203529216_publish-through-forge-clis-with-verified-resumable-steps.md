+++
schema_version = 1
id = "01M3FPW5T0FAQ8B1FRTR421XWH"
title = "Publish through forge CLIs with verified resumable steps"
date = "2026-09-26"
status = "accepted"
tags = ["ux", "safety"]
supersedes = []
superseded_by = []
depends_on = []
related_to = []
+++
## Decision

Support GitHub through gh and GitLab through glab. Require an explicit owner/name and public/private visibility. Publishing preflights the checkout and destination, creates a remote, verifies identity and visibility, pushes one immutable current-branch tip without force, verifies it remotely, then moves the checkout. An explicit resume mode continues a matching origin after partial failure.

## Why

The user delegated forge and UX choices in the build brief. Forge CLIs reuse existing authentication. Explicit visibility makes the external publication decision visible at invocation; Claude's UX review highlighted partial failure and recovery.

## Consequences

The relevant authenticated forge CLI is required. Dry-run performs read-only checks but never creates, pushes or moves. Failures preserve existing remote/local progress and print recovery guidance; no rollback deletes a repository. repot sync still never pushes.
