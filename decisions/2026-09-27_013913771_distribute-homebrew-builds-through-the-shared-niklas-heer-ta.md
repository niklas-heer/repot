+++
schema_version = 1
id = "01M3G88B7BQ578QPRKXW277H5J"
title = "Distribute Homebrew builds through the shared niklas-heer tap"
date = "2026-09-27"
status = "accepted"
tags = ["packaging", "release"]
supersedes = []
superseded_by = []
depends_on = []
related_to = ["01M3FR7K9EG53T35NX42JW74KV"]
+++
## Decision

Publish repot's Homebrew formula in the shared `niklas-heer/homebrew-tap`
(`brew install niklas-heer/tap/repot`) instead of a formula in this repository.
Releases publish the four native archives and a `SHA256SUMS` covering exactly
those archives. The tap's **Update repot** workflow reads the latest stable
release hourly, renders the formula from `SHA256SUMS`, installs and tests it on
macOS and Linux for ARM64 and x86-64, and opens a pull request; merging it
publishes the update. The tap formula keeps `--HEAD` source builds.

This repository keeps a `tap_migrations.json` that points the former
`niklas-heer/repot` tap to `niklas-heer/tap`, so existing installations move over
on `brew update`.

## Context

On 2026-09-27 the user asked to install every one of their apps from their
Homebrew tap rather than a tap per repository. The tap already distributes vrdx
this way: a generator script with tests, a checksum-verified candidate and
native installation tests before a reviewed pull request. Generating the formula
in the release workflow as well would keep two templates that could drift.

## Consequences

Users need one tap for all tools, and repot releases no longer need a manual
formula commit. A new version reaches Homebrew only after the tap workflow runs
and its pull request is merged, so there can be up to an hour's delay plus
review. The release workflow no longer installs a Homebrew formula itself; the
tap's installation matrix replaces that gate and runs against the published
assets. Release asset names and the `SHA256SUMS` format are now an interface
with the tap generator, and changing them requires updating the tap.
