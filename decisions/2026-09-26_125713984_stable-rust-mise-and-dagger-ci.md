+++
schema_version = 1
id = "01M3EWN300JV9MM34A8C6NK2CD"
title = "Stable Rust, mise and Dagger CI"
date = "2026-09-26"
status = "accepted"
tags = ["tooling", "ci", "license"]
+++

## Decision

repot uses pinned stable Rust with strict Clippy, mise for tools and tasks, nextest for tests, Dagger with Dang for Linux CI alongside a native macOS job, and is public under the MIT license.

## Why

These are the owner's defaults for new Rust tools, already proven in sibling projects, and give one set of local commands that CI reuses.

## Consequences

- mise run ci-native runs the same gates locally and inside Dagger.
- Contributors need mise, and a container engine for mise run ci.
- Strict lints cost some ceremony in exchange for no unwrap, panic or unchecked arithmetic in production code.
