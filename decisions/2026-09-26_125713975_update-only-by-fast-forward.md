+++
schema_version = 1
id = "01M3EWN2ZQRTCDMNKC2ZCS4H9V"
title = "Update only by fast-forward"
date = "2026-09-26"
status = "accepted"
tags = ["safety", "git"]
+++

## Decision

repot updates repositories only by fast-forward, and switches back to the default branch only when the current branch is clean, has nothing local-only, and its work is already upstream; it never commits, stashes, resets, rebases or discards work.

## Why

The owner wants all repositories current without risking merge conflicts or lost work, and wants to leave branches that were already merged on GitHub.

## Consequences

- Bulk sync is safe to run at any time, including while agents work in other checkouts.
- Dirty, diverged, detached or ambiguous repositories are reported for a person to resolve.
- Squash-merge detection needs extra signals such as pruned upstreams, patch identity or the forge's pull request state.
