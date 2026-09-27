+++
schema_version = 1
id = "01M3HEJGRTDQM7EQ3KFRVEJ287"
title = "Rank the picker by frecency from a local visit log"
date = "2026-09-27"
status = "accepted"
tags = ["ux", "storage"]
supersedes = []
superseded_by = []
depends_on = []
related_to = []
+++
## Decision

Rank the picker by frecency and Git activity. Record repository visits in a
local append-only text log at `$XDG_STATE_HOME/repot/visits`, one
`<unix seconds>\t<checkout path>` line per switch, with `%`, tab, newline and
carriage return percent-encoded. Shell integration adds a directory-change hook
for bash, zsh, fish and Nushell that calls a hidden `repot visit PATH`; `repot cd`
also records its selection. A visit is only recorded for the enclosing checkout
and only when it differs from the previous entry. The log is compacted to the
latest 4,000 visits by atomic rename. `shell-init --no-track` omits the hook.

Weights per visit are 16 within an hour, 8 within a day, 4 within a week, 2
within a month and 1 otherwise. With an empty query the picker orders by that
sum, then by the newest modification time of the checkout's index, HEAD reflog
or HEAD. With a query, fuzzy matching decides and a boost of `4 * sqrt(frecency)`
refines it without ever admitting non-matches.

## Context

On 2026-09-27 the user asked for the picker to put the most recently touched and
most frequently used repositories first, and asked whether a SQLite file would
be needed. Directory switches are small, append-mostly events. A single
`O_APPEND` write per visit is safe across concurrent shells, needs no schema,
migration or C dependency, stays readable and can simply be deleted. SQLite
(through rusqlite with a bundled build) would add a native build step and a
database for data that fits in half a megabyte. zoxide's approach of hooking
directory changes was adopted because selections made only through the picker
would miss how people actually move around.

## Consequences

Ranking reflects real use from the first day the hook is active. Every directory
change spawns a short-lived process; `visit` skips configuration loading and
stops after a few `stat` calls and one append. A visit written during compaction
can be lost, which only nudges ranking. Git activity also changes when repot
itself fast-forwards a checkout, so recently synced checkouts can rise among
unvisited ones. The file holds local paths only and never leaves the machine.
Revisit the storage if ranking needs richer queries than per-path aggregates.
