+++
schema_version = 1
id = "01M3FTDH8X48NY3C999BG2SV5T"
title = "Embed the Rust fuzzy picker in the binary"
date = "2026-09-26"
status = "accepted"
tags = ["ux"]
supersedes = ["01M3FNVVATYBSP10J7Q1NW3JX4"]
superseded_by = []
depends_on = []
related_to = []
+++
## Decision

Embed the fuzzy picker using Nucleo, Ratatui and Crossterm. Keep deterministic direct selection for exact or unique queries and reject ambiguous noninteractive selection. Use the controlling terminal so Nushell can capture command output without breaking interaction.

## Context

On 2026-09-26 the user explicitly rejected the external fzf dependency and requested a self-contained interface. The previous optional-fzf decision is superseded.

## Consequences

No external picker is required. Rust dependencies and binary size grow in exchange for controlled matching, rendering and input behavior. Tests must cover real PTYs, terminal restoration, all supported shell handoffs and reproducible input sequences. Git remains required for repository operations; this decision concerns the interface.

## Verified implementation notes

With Ratatui 0.30.2, `Terminal::clear` queries the cursor through global stdout,
even when rendering through `/dev/tty`. Recreate the terminal after suspension
instead, so redirected output and Nushell capture remain usable. Initialize the
Crossterm event source before the first frame and compare rendered dimensions
against the terminal size to handle a resize during startup. These behaviors are
covered by [real PTY tests](../tests/picker.rs); recheck them when upgrading the
terminal dependencies.

The first release rehearsal exposed stalled keyboard input after resizing on
both macOS and Linux Nix. Crossterm 0.29's Mio backend can discard a ready keyboard
event when it returns a resize event from the same poll batch. Enable its
`use-dev-tty` backend, which uses level-triggered Unix polling. The original
failure test and a repeated resize/edit regression run without input retries;
1,600 coordinated cycles passed locally after this change.
