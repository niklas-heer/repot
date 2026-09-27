+++
schema_version = 1
id = "01M3HFF634V9R3WN9MSKRDMNE4"
title = "Batch GitHub branch state before fetching"
date = "2026-09-27"
status = "accepted"
tags = ["performance", "network"]
supersedes = []
superseded_by = []
depends_on = []
related_to = []
+++
## Decision

Before fetching, ask GitHub once per run, through `gh api graphql`, for every
GitHub-hosted checkout's default branch, the tip of its upstream branch (or its
absence) and, for feature branches, the merge state of a matching pull request.
Skip `git fetch` for a checkout whose remote tip equals its local tracking ref,
and drop the per-repository `ls-remote --symref` and `gh pr view` calls. Fall back
to the normal fetch for any mismatch, error, missing `gh` authentication or
non-GitHub remote. Sync keeps revalidating every plan against local refs before
changing anything, and all mutations stay on the Git CLI.

## Context

On 2026-09-27, with 45 real checkouts, fetching dominated `repot status`: every
SSH connection to GitHub costs about 1.2 s through the SSH agent, so even at 24
in parallel status takes about 5 s. A probe answered branch tips and default
branches for several repositories in one 0.5 s request, and resolved a renamed
repository to its new name. Most checkouts are unchanged between runs, so most
fetches only confirm what is already known.

`remote.<name>.followRemoteHEAD` was evaluated as a way to learn the default
branch during the fetch. It only applies when fetching with the remote's
configured refspecs; repot passes an explicit refspec with `--refmap=` so that
configured mappings can never update local branches, so it cannot be used.

## Consequences

Expected status time with network drops from about 5 s to about 1.5 s when few
repositories changed. Skipped checkouts do not prune or update their other
remote-tracking branches, and tags are not fetched either way. Correctness for
the current branch rests on GitHub reporting the same tip the remote serves; a
stale answer can only cause a skipped update that the next run picks up, because
plans are still built from and revalidated against local refs. The feature
depends on `gh` being installed and authenticated and on GitHub's GraphQL rate
limits (one query per run). Other forges keep the current behaviour.

## Implementation notes

The user agreed on 2026-09-27. Measured on the same 45 checkouts: 17 s before
the connection work, about 5 s with one connection per checkout, and 2.1-2.4 s
with the survey, with status JSON identical to a run where GitHub is
unreachable. Three details mattered:

- The merged-pull-request lookup made one batch take 3 s on large repositories.
  It is only requested when the checkout is not on its cached default branch.
- Batches of ten run in parallel (up to eight at once) instead of one large
  query.
- Checkouts GitHub cannot answer for (other hosts, no upstream branch) are
  inspected first, so their fetches overlap with the query; the others wait for
  the answers, which are always published even if the query thread fails.

The fetch is skipped only when both the upstream tip and the default-branch tip
match the local tracking refs for the remote the inspection uses. The terminal
report states how many checkouts GitHub confirmed.

