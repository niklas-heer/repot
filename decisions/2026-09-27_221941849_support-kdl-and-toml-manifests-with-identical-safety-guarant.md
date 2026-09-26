+++
schema_version = 1
id = "01M3FWTZWSMF8DYJPQBVPRKY76"
title = "Support KDL and TOML manifests with identical safety guarantees"
date = "2026-09-27"
status = "accepted"
tags = ["architecture", "configuration"]
supersedes = []
superseded_by = []
depends_on = []
related_to = ["01M3FQFW55W1EVXPSMJ2ZZHMX7"]
+++
## Decision

Support KDL v2 alongside TOML for repository manifests, with the same data model
and safety guarantees. Select KDL by the logical `.kdl` filename; preserve TOML
handling for existing filenames. Discover either `repos.toml` or `repos.kdl`,
require explicit selection when both exist, and keep TOML as the new-file default.

## Context

On 2026-09-27 the user requested KDL in addition to the existing manifest format.
Use the maintained Rust `kdl` parser with minimal features and syntax-tree edits,
extending the existing portable-manifest decision. Strict schema validation and
generic parse errors prevent unnoticed mistakes and credential disclosure.

## Consequences

Both formats retain comments, dotfiles symlinks, atomic writes and concurrent
registration protection. A second parser adds dependency and testing cost; real
CLI parity and seeded editing tests must cover both formats. No format conversion
or merged configuration precedence is implicit.
