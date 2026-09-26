+++
schema_version = 1
id = "01M3FTDH8YFZB18074SNK3K7SY"
title = "Replace ghq Git workflows while preserving work"
date = "2026-09-26"
status = "accepted"
tags = ["scope", "safety"]
supersedes = []
superseded_by = []
depends_on = []
related_to = ["01M3EWN2ZDBN2ZWZHBT6SB41Y2", "01M3EWN2ZQRTCDMNKC2ZCS4H9V"]
+++
## Decision

Replace ghq's Git workflows while retaining its directory layout and configuration. Add cloning/import, root and list compatibility, in-tree creation and migration. Repository removal archives work atomically and exposes explicit restoration rather than permanently deleting it.

## Context

On 2026-09-26 the user explicitly changed the goal from supplementing ghq to replacing it, then confirmed full Git workflows rather than legacy version-control backends. The established no-data-loss rules still apply.

## Consequences

ghq is no longer needed. Existing trees and root settings remain valid. Compatibility changes must be tested against ghq 1.10.1 and documented when safety requires different behavior. Archives preserve dirty, ignored, staged and committed work; unsupported dependent layouts require manual handling.
