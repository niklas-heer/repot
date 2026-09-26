+++
schema_version = 1
id = "01M3FYDYDS9VRR5FAG3Y5JZWFS"
title = "Support TOML KDL and YAML manifests with shared safety guarantees"
date = "2026-09-27"
status = "accepted"
tags = ["architecture", "configuration"]
supersedes = ["01M3FWTZWSMF8DYJPQBVPRKY76"]
superseded_by = []
depends_on = []
related_to = ["01M3FQFW55W1EVXPSMJ2ZZHMX7"]
+++
## Decision

Support TOML, KDL v2 and YAML manifests with one data model and identical write
safety. Logical `.kdl`, `.yaml` and `.yml` suffixes select their parsers; other
filenames retain TOML compatibility. Discover `repos.toml`, `repos.kdl`,
`repos.yaml` or `repos.yml`, requiring explicit selection if several exist.

## Context

On 2026-09-27 the user explicitly clarified that all three formats are required.
Use `kdl` and `yaml-edit` with minimal features alongside `toml_edit` to preserve
human-maintained comments. YAML accepts concrete single-document mappings and
rejects directives, tags, aliases and anchors so edits remain local and explicit.

## Consequences

TOML remains the new-file default. Every format uses the existing symlink-aware
lock, reread and atomic write path. Strict validation and source-free parse errors
apply throughout. No merging or conversion is implicit. CLI parity, concurrent
registration and seeded editing tests must cover all three formats.

## Verified dependency correction

The `yaml-edit` 0.3.2 scalar classifier panics on a valid plain string containing
two minus signs followed by the minimum signed integer. Its runtime sources are
vendored with a single checked-negation correction, provenance and Apache 2.0
license in [vendor/yaml-edit](../vendor/yaml-edit/REPOT_PATCH.md). The binary
regression covers quoted and unquoted forms. Replace the local patch once an
upstream release passes those tests. Git source, Nix and binary archives are the
release channels; crates.io publication is disabled while this path dependency
is required.
