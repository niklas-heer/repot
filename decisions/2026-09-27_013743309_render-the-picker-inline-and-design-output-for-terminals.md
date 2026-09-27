+++
schema_version = 1
id = "01M3G85JWDQMRDZ26AZ0WZHZR7"
title = "Render the picker inline and design output for terminals"
date = "2026-09-27"
status = "accepted"
tags = ["ux"]
supersedes = []
superseded_by = []
depends_on = []
related_to = ["01M3FTDH8X48NY3C999BG2SV5T"]
+++
## Decision

Keep the embedded picker, but render it inline in a Ratatui `Viewport::Inline`
anchored under the prompt instead of on the alternate screen. It is at most
twelve rows tall, never taller than the results need, and it erases itself on
selection, cancellation or suspension so the prompt returns where it was. While
the picker runs, stdout is pointed at the controlling terminal and restored
before the selection is printed.

Design human output for terminals and keep machine output stable:
`status` and `sync` group checkouts by the action they need (failed, needs
review, ready to push, ready to update or updated, up to date) with compact
facts per row, a per-owner summary of healthy checkouts and one suggested next
step. A spinner with a counter runs on an interactive stderr during fetches,
clones and updates. Errors print as `error:` with an optional `hint:`, and
`repot --help` opens with a small logo, the version and task-grouped commands.
Piped text keeps the tab-separated format and `--json` is unchanged. `NO_COLOR`
and `CLICOLOR_FORCE` are honoured.

The Nushell wrapper passes report commands straight through, so they stay
attached to the terminal and pipeable, and runs relocation commands without
capturing their output.

## Context

On 2026-09-27 the user found `repot status` hard to read and asked for inline
fuzzy search that does not take over the screen, since switching repositories
should not hide the status they were just reading. The raw status output came
from the Nushell wrapper piping repot through `tee | complete`, which made
stdout a pipe and also buffered everything until exit, so no progress could
show.

An inline viewport needs the cursor position. Crossterm 0.29 writes that query
to the process's stdout, not to the terminal the UI renders on, so a redirected
stdout (as in `cd "$(repot cd)"` or the tests) would receive the query and never
answer. Redirecting fd 1 to `/dev/tty` for the picker's lifetime keeps
Crossterm's own response parsing and its queueing of typed-ahead keys.

## Consequences

Scrollback stays visible and navigation keeps the user's flow. The picker
depends on the terminal answering cursor position reports; a terminal that never
answers gets an error after Crossterm's two-second timeout instead of a picker.
PTY tests must answer those reports like a terminal emulator. The stdout
redirection must be restored before any output is written, and suspension must
recreate the viewport. Human-readable layouts may keep evolving; scripts should
use `--json` or piped text.
