+++
schema_version = 1
id = "01M3FNVVATYBSP10J7Q1NW3JX4"
title = "Use deterministic selection with optional fzf"
date = "2026-09-26"
status = "superseded"
tags = ["ux"]
supersedes = []
superseded_by = ["01M3FTDH8X48NY3C999BG2SV5T"]
depends_on = []
related_to = []
+++
## Decision

Use deterministic built-in subsequence filtering and optional fzf for interactive ambiguous selection. Exact and unique matches work without fzf. Ambiguous noninteractive requests fail. Shell wrappers use a temporary file directory handoff for bash, zsh, fish and Nushell.

## Why

This supports both scripts and interactive navigation without a terminal UI dependency. It follows the implementation scope delegated on 2026-09-26 and the Claude Opus 5.5 UX review.

## Consequences

Interactive ambiguity requires fzf; install instructions must explain this. Only a successful command may change the parent shell directory. Paths must survive spaces and newline characters, and candidate ordering must be stable.
