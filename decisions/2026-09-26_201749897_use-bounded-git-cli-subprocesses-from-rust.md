+++
schema_version = 1
id = "01M3FNVVA9SKEFPTXZVADZGXPC"
title = "Use bounded Git CLI subprocesses from Rust"
date = "2026-09-26"
status = "accepted"
tags = ["architecture", "git"]
supersedes = []
superseded_by = []
depends_on = []
related_to = []
+++
## Decision

Use the Git CLI from Rust with bounded subprocesses, standard-library concurrency and Unix process-group cancellation. Use Serde for JSON, TOML for the manifest, and tempfile for atomic staged writes and isolated tests. Do not add a Git library or asynchronous runtime.

## Why

Git already honors credential helpers, SSH and user configuration. The user delegated implementation of the build brief on 2026-09-26. Bounded concurrency suits a few dozen repositories; typed Rust reports separate inspection from safe actions. Nix signal bindings provide cancellation without unsafe application code.

## Consequences

Git is a runtime requirement. Linux and macOS are the initial release platforms. Child diagnostics must not expose remote credentials. Tests run the binary against isolated real Git repositories and deterministic action sequences.
