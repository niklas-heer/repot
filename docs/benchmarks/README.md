# Native Git read experiment

The first native fast path resolves the private and shared Git directories with
`gix` and checks the eight operation markers directly. This removes eight Git
subprocesses per clean repository in status/sync inspection. Reference handling,
porcelain status, ancestry, fetching and checkout mutations still use the bounded
Git runner and its existing safety checks. A native opener or filesystem failure
falls back to Git.

`gix = 0.88.0` uses `default-features = false` with only `sha1` and `sha256`.
The same dependency also supports native configuration reading. No network,
credential, index-status or worktree-mutation features are enabled.

## Reproduce

Build optimized binaries and run the isolated harness:

```sh
mise exec -- cargo build --release --example layout_bench --bin repot
python3 scripts/benchmark.py --root-mode env \
  --layout-bench target/release/examples/layout_bench --output /tmp/repot-env.json
python3 scripts/benchmark.py --root-mode config \
  --layout-bench target/release/examples/layout_bench --output /tmp/repot-config.json
```

For a status comparison, first build an earlier revision in a separate worktree
and pass its executable as `--baseline /path/to/earlier/repot`. The checked-in
measurements use baseline revision `e037a668af77c1007f85233c2a1decc29d126175` from the preceding completed milestone.
Pass `--baseline-revision e037a66` to identify that source; the JSON also records
SHA-256 digests of the actual executables. `--root-mode env` supplies `GHQ_ROOT`;
`--root-mode config` instead supplies `ghq.root` through isolated Git configuration.
The harness never modifies the repository or the real home directory. It creates
24 clones of a local bare remote, each with 16 small tracked files and the same
fixed commit, under a temporary home. It removes inherited Git configuration
overrides, warms each command twice, interleaves nine measured repetitions and
asserts output parity. No network, credentials or cold-cache eviction are used.

`ghq list --full-path` and `repot list --full-path` are the comparable operations.
`ghq` has no equivalent to repot's complete status report, so status compares
repot versions. The optional Rust example isolates the exact marker checks and
asserts identical Git/native results. Its Git path uses direct subprocesses,
without the production timeout runner's polling overhead. Full executable timings
include startup and output capture. Results describe one synthetic warm-cache
corpus on one machine; they are not a universal performance claim.

## Measured results

Final report prepared on 2026-09-27 Europe/Berlin, after build/test workloads
stopped. Each JSON records the measurement completion timestamp in UTC.
Raw samples and executable SHA-256 digests are recorded separately for
[`GHQ_ROOT`](2026-09-27-macos-arm64-env.json) and
[Git configuration](2026-09-27-macos-arm64-config.json).

| Root selection | repot list | ghq list | repot status | Baseline status |
| --- | ---: | ---: | ---: | ---: |
| `GHQ_ROOT` | 3.44 ms | 5.05 ms | 576 ms | 888 ms |
| Git `ghq.root` | 7.15 ms | 12.65 ms | 580 ms | 901 ms |

The isolated marker check measured **3.519 ms native versus 669.329 ms Git**
for all 24 repositories in the environment-root run. This measures identical
marker queries, not complete Git status work. Git subprocess startup dominates
that narrow comparison.

The optimized repot executable was 6,982,800 bytes, versus 1,918,928 bytes for the
baseline and 9,504,322 bytes for installed ghq. The repot increase includes the
embedded picker, TLS/vanity resolution, completions and other workflow additions;
it does not isolate gix's binary-size cost. No strip or compression normalization
was applied to these executable sizes.

## Correctness and compatibility

`tests/git_read.rs` checks each marker path against Git for SHA-1 and SHA-256
repositories, packed refs, detached HEAD, linked worktrees, a worktree path ending
in a newline, shared grafts and separate Git directories. Public binary tests
verify operation refusal and invalid configuration errors. Existing process-fault,
status and deterministic simulation tests continue to exercise Git's authoritative
checks and the mutation backend.

A useful finding: gix's strict opener can accept an unknown repository extension
which Git refuses. Therefore opening a repository successfully is not treated as
proof that all Git operations are supported. Git's own status inspection still
runs before this layout optimization, and the regression test requires refusal.

## Current upstream readiness

The [gix 0.88 API and feature documentation](https://docs.rs/gix/0.88.0/gix/)
provides native repository/configuration/object access and optional status and
transport APIs. Its documented library guidance is to disable default features
and enable only needed components. SHA-256 is an explicit feature.

The [upstream capability inventory](https://github.com/GitoxideLabs/gitoxide/blob/main/crate-status.md)
was reviewed on 2026-09-26 alongside the versioned API and source. Clone/fetch,
credential-helper integration and worktree inspection have substantial support;
complete command-level workflow coverage remains uneven. The inventory itself
labels its workflow summary as generated, so it is a starting point for checking
actual APIs, not an independent compatibility guarantee.

In particular, the [0.88 push module](https://docs.rs/gix/0.88.0/gix/push/index.html)
is empty. A native push replacement is not available through that public API.
Existing-checkout switching and merge orchestration must still be validated
against repot's ignored-file, hidden-index-flag, submodule, linked-worktree,
no-autostash and exact-ref invariants before migration. SSH/file transports can
spawn external programs, so enabling native clone does not by itself deliver a
fully standalone binary. No such claim is made here.
