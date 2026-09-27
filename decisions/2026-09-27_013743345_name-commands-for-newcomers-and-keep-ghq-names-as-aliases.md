+++
schema_version = 1
id = "01M3G85JXH0ZYAK7ZDK6G8ZAJM"
title = "Name commands for newcomers and keep ghq names as aliases"
date = "2026-09-27"
status = "accepted"
tags = ["ux", "cli"]
supersedes = []
superseded_by = []
depends_on = []
related_to = []
+++
## Decision

Name commands after what someone new wants to do, and keep the ghq-style names
working:

- `cd` jumps to a repository; `jump` remains a hidden alias.
- `clone` clones; `get` remains an alias.
- `new NAME` starts a scratch project, and `new OWNER/NAME` (or host/owner/name,
  or a URL) creates an empty repository at its tree location. `create` remains a
  hidden command with its ghq semantics.
- `scan` finds repositories outside the tree; `find` remains an alias.
- `migrate` is hidden; `adopt` covers it.
- `list` also answers to `ls`.

The help overview groups commands by task: get around, stay current, bring
repositories in, clean up, set up. MCP tool names are unchanged.

## Context

On 2026-09-27 the user asked for command names that make the most sense to a
newcomer, questioning `jump` versus `cd`, and delegated the choice. Two pairs
were near-synonyms that forced people to learn ghq's history: `new`/`create` and
`adopt`/`migrate`. `find` suggested searching known repositories, which is what
`list QUERY` and `cd` do. `repot cd` reads as what it does through the shell
wrapper, and without the wrapper it still prints the path for
`cd "$(repot cd query)"`. `shell-init` stays explicit because `init` would read
as creating a repository.

## Consequences

Existing scripts, completions habits and agent calls keep working through the
aliases and hidden commands. Documentation and completions show only the new
names. `new` treats any name containing `/` or `:` as a tree location, so a
scratch project can never contain a slash, which was already invalid. Revisit if
an alias conflicts with a future command.
