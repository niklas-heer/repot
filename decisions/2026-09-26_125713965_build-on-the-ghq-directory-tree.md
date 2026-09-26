+++
schema_version = 1
id = "01M3EWN2ZDBN2ZWZHBT6SB41Y2"
title = "Build on the ghq directory tree"
date = "2026-09-26"
status = "accepted"
tags = ["architecture", "scope"]
+++

## Decision

repot is its own Rust CLI that reads and extends the ghq layout (<root>/<host>/<owner>/<repo>) and its root configuration instead of replacing it with a new layout.

## Why

The owner likes the tree and already keeps a few dozen checkouts in it. Existing tools either replace ghq's layout (git-repo-manager), mirror whole forge accounts (git-workspace), or lack sync and discovery (ghr), and none handles squash-merged branches or scratch projects.

## Consequences

- ghq keeps working alongside repot, and no checkout has to move to adopt it.
- repot must follow ghq's root resolution (GHQ_ROOT, ghq.root, multiple roots).
- Repositories outside the tree need explicit registration or adoption.
