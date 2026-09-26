+++
schema_version = 1
id = "01M3FR7K9EG53T35NX42JW74KV"
title = "Build verified native releases and locked source packages"
date = "2026-09-26"
status = "accepted"
tags = ["packaging", "ci"]
supersedes = []
superseded_by = []
depends_on = []
related_to = []
+++
## Decision

Build release archives natively for ARM64 and x86-64 Linux and macOS. Run the existing Dagger and native macOS quality gates plus a locked source-built Nix check before release packaging. Publish only on a version tag matching the executable. Generate the stable Homebrew formula from the four actual archives; retain a HEAD source formula for development. Pin Nix inputs and derive its Rust toolchain version from rust-toolchain.toml.

## Why

The user delegated implementation of all five build-brief milestones on 2026-09-26, including release packaging. Native target runners avoid an extra cross-compilation toolchain. Asset-derived checksums cannot claim hashes for artifacts that do not exist. The Nix build checks runtime and test dependencies in isolation and runs all four shell integrations.

## Consequences

Local package tests exercise archive extraction, the actual executable, checksums and architecture-specific formula URLs. Linux GNU binaries target the release runner's supported runtime; older Linux distributions can use the source-built Nix package. Manual workflow dispatch verifies and produces artifacts without publishing. Implementing this workflow does not itself create a tag, release or external Homebrew tap entry.
