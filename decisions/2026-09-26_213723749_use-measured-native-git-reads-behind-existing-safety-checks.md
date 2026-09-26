+++
schema_version = 1
id = "01M3FTDH95X899C1P70XP7RA91"
title = "Use measured native Git reads behind existing safety checks"
date = "2026-09-26"
status = "accepted"
tags = ["architecture", "performance"]
supersedes = ["01M3FNVVA9SKEFPTXZVADZGXPC"]
superseded_by = []
depends_on = []
related_to = []
+++
## Decision

Use gix 0.88 with minimal SHA-1/SHA-256 features for verified read-only Git metadata paths. Keep Git CLI authority for status, refs, authentication, network operations and every mutation, and fall back when native layout recognition declines.

## Context

The user delegated Rust Git performance experiments on 2026-09-26. Differential tests cover normal, linked, separate, corrupt and SHA-256 layouts. The [reproducible 24-repository measurements](../docs/benchmarks/README.md) compare ghq listing, baseline/current status with identical JSON, and isolated native/Git operation-marker checks. Full-command results include other feature changes, so they do not isolate gix's contribution. gix does not yet supply the entire safe checkout/push workflow.

## Consequences

Git remains a runtime dependency. Native reads must not relax existing safety checks. Added crate and binary costs are measured, and future migrations require parity tests and reproducible evidence. Measurements and limitations live in docs/benchmarks/.
