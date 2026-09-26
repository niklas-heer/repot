<p align="center"><img src="assets/repot.png" width="112" alt="repot: a Git branch sprouting from a terracotta pot"></p>

# repot

Keep every Git repository on a machine organised, current and portable.

repot is a Rust Git repository organiser and a replacement for ghq's Git workflows.
Clone, find and jump between projects, safely update your checkouts, publish a
scratch project, or restore a machine from your dotfiles. It keeps the familiar
`<root>/<host>/<owner>/<repo>` tree and existing ghq configuration.

> **Status:** all six implementation milestones are complete, including the Git-focused ghq replacement, embedded picker, measured native Git reads and project icon. See [BUILD_BRIEF.md](BUILD_BRIEF.md) for safety rules and scope.

## Install

repot supports Linux and macOS on Intel/AMD and ARM64. The fuzzy picker is built
into the binary: neither ghq nor fzf is required. Git handles network operations,
authentication and checkout changes; install `gh` or `glab` only for publishing
and optional forge-specific merge evidence. Nix and Homebrew supply Git.

From a checkout with the pinned Rust toolchain:

```sh
mise exec -- cargo install --locked --path .
```

With Nix, the locked flake builds from source and supplies Git at runtime:

```sh
nix build .
nix run . -- --help
```

The release workflow produces a tarball for each supported platform, `SHA256SUMS`,
and a Homebrew formula with hashes of the actual archives. Verify the checksums
before extracting a downloaded archive and placing `repot` on your `PATH`.
`Formula/repot.rb` is a source-build HEAD formula for a Homebrew tap; a stable tap
can use the generated `repot.rb` release asset. No tap or release is published by
a local build.

To install the current source through Homebrew, use this repository as a tap:

```sh
brew tap niklas-heer/repot https://github.com/niklas-heer/repot
brew install --HEAD niklas-heer/repot/repot
```

## Commands

| Command | Purpose |
| --- | --- |
| `repot get` / `repot clone` | Clone one or many repositories; safely update with `-u` |
| `repot root [--all]` | Show primary or all configured roots |
| `repot list [--json]` | List discovered and registered repositories |
| `repot jump` | Fuzzy-pick a repository and `cd` into it |
| `repot status` / `repot sync` | See every repository's state; fast-forward what is safe |
| `repot new` / `repot publish` | Start a local scratch project; later create its remote and move it into the tree |
| `repot find` / `repot adopt` | Discover repositories outside the tree and bring them in |
| `repot restore` | Clone missing entries from your manifest on a new machine |
| `repot create` | Create an empty Git repository directly in the host/owner/name tree |
| `repot migrate` | Move and register an existing checkout |
| `repot rm` / `repot trash` | Archive a checkout without losing work; list or restore archives |
| `repot shell-init <shell>` | Print shell integration |
| `repot completions <shell>` | Generate bash, zsh, fish or Nushell completions |

repot never commits, stashes, resets or force-pushes, and only updates a repository by fast-forward.

## Discovery and navigation

Roots follow ghq: `GHQ_ROOT`, then all Git `ghq.root` values, then `~/ghq`.
`GHQ_ROOT` accepts a colon-separated path list on Unix, with its first root primary;
otherwise the last `ghq.root` value is primary. URL-specific `ghq.<url>.root`
settings are supported and existing checkouts are reused across roots. Registered
manifest paths are included too. The manifest defaults to `$XDG_CONFIG_HOME/repot/repos.toml`
(or `~/.config/repot/repos.toml`); override it with `--manifest PATH`.

```sh
repot list --json
repot list -p --exact repot
repot jump repot
# Add the appropriate integration to your shell configuration:
eval "$(repot shell-init bash)"    # use zsh in zsh
repot shell-init fish | source    # fish
# Nushell: save `repot shell-init nu` to a file and source that file.
# Optional completions (save/source using your shell's normal convention):
repot completions nu
```

The embedded picker uses Nucleo matching and a Ratatui interface. Type to filter,
use arrows or Ctrl+N/P to choose, Enter to jump, Escape or Ctrl+C to cancel, and
Ctrl+U to clear the query. It highlights matches, shows the selected full path,
handles resizing and respects `NO_COLOR`. Exact and unique matches jump directly;
noninteractive scripts must provide an unambiguous query. Without the shell
wrapper, `jump` prints the selected path. Directory symlinks are not traversed;
linked worktrees can be discovered or registered, and nested submodules are not
listed separately.

`list` prints root-relative paths by default, `-p` prints absolute paths, `-e`
matches exact suffixes, and `--unique` prints the shortest unambiguous names.
JSON retains absolute paths. Bare repositories appear in `list`; bulk status and
sync operate on working checkouts. `root --all` prints roots in priority order.

## Replace ghq

Existing checkouts stay where they are. Use repot for the same Git tasks:

```sh
repot get owner/project
repot get -p github.com/owner/project       # SSH
repot get --shallow --branch main owner/project
repot get --partial blobless owner/large-project
repot get --bare owner/project
repot get --update owner/project           # fast-forward only
repot list | repot get --parallel          # newline-separated import
repot get --look owner/project             # cd through shell-init
repot create owner/new-project             # init only; no remote or commit
repot migrate ~/Downloads/project -y --dry-run
```

`get` also accepts multiple arguments, `repo@branch`, `--partial treeless`,
`--silent`, `--no-recursive`, and `--vcs git` (or `github`). Short repository
names honor `ghq.user`, `github.user`, `ghq.completeUser`, and `ghq.defaultHost`.
New clones include submodules by default; `--no-recursive` disables that.
Clones stage privately before an atomic no-overwrite move. Parallel imports use
bounded workers and report failures while completing independent repositories.
`--json` provides structured results and `--timeout` bounds Git operations.

GitHub webpage paths resolve to their repository root; GitLab subgroup paths stay
intact. SCP-style SSH, HTTP(S) and `git://` URLs are supported. Failed HTTP(S)
clones can resolve Git `go-import` metadata with bounded requests, no redirects,
and verified import prefixes. `--vcs git` or URL-scoped `ghq.<url>.vcs = git`
bypasses that fallback. A dry-run does not contact a vanity host, so its final
destination can remain unresolved until cloning.

AWS `codecommit://[profile@]repo` and `codecommit::region://[profile@]repo` retain
ghq's region/repository layout. Region selection uses the explicit URL, then
`AWS_REGION`, `AWS_DEFAULT_REGION`, and finally bounded `aws configure get region`.
These optional cloud URLs require the Git CodeCommit helper for transport. The
AWS CLI is needed only for the fallback region lookup. Ordinary Git hosting
requires neither. Credentials belong in Git/AWS helpers,
never in repository URLs or repot's manifest.

Every mutating command supports `--dry-run`. Updates use repot's safety checks:
dirty, diverged, linked/shared or submodule checkouts may require review. `get -u`
does not switch branches; use `sync` for proven merged-branch return. Bare updates
fetch refs atomically without forcing or pruning. Creation refuses even an empty
existing directory. The target is Git workflows, not ghq's legacy VCS backends.

Removal preserves work instead of deleting it permanently:

```sh
repot rm owner/old-project --dry-run
repot rm owner/old-project
repot trash list
repot trash restore repo-ARCHIVE_ID
```

Archives live in `.repot-trash` under the relevant root and are excluded from
normal discovery. Dirty files, ignored files, commits, stashes and index contents
move together. Restoration refuses an occupied destination. Both operations
require a standalone checkout on the same filesystem; dependent worktrees and
object databases require manual handling.

Path components beginning with `.repot-clone-`, `.repot-new-`, `.repot-create-`
or `.repot-restore-`, and the exact component `.repot-trash`, are reserved for
internal staging and archives. Creation, import and registration reject these
names, including in dry-runs, so a successful operation remains discoverable.

## Status and safe updates

```sh
repot status --json
repot sync --dry-run
repot sync --jobs 4 --timeout 30
```

Status fetches the relevant remote for each checkout with bounded parallelism.
`--no-fetch` or `status --dry-run` uses cached tracking refs. `sync --dry-run` also uses cached refs and
prints that limitation; it does not fetch, write refs, or change the working tree.
JSON is a sorted array with `path`, `state`, `action`, `branch`, dirty counts,
stash count, ahead/behind counts, reason, and whether an action was applied.

Sync only fast-forwards or returns a proven merged branch to the remote default.
It never pushes, deletes branches, commits, stashes, resets, or rebases. Dirty,
detached, diverged, unconfigured-upstream, submodule, and in-progress-operation
checkouts need review. Branch return checks ancestry, patch equivalence after
pruning, or an exact merged GitHub PR head and default base through optional `gh`.
An ahead/diverged default branch or one checked out in another worktree blocks
return. Existing feature branches and their commits are retained.

Exit codes: `0` completed, `1` operational failure, `2` invalid usage, `3` manual
review or push needed. Failures take precedence over review in a mixed batch.

## Scratch projects and publishing

```sh
repot new experiment                  # <primary-root>/local/scratch/experiment
repot new experiment --namespace me --dry-run
# Commit manually when ready, then preview and publish:
repot publish owner/experiment --visibility public --dry-run
repot publish owner/experiment --visibility public
repot publish group/project --forge gitlab --visibility private
# Use --host gitlab.example.com for a self-hosted forge.
```

Publishing requires a clean committed branch, an authenticated `gh` or `glab`, an
explicit target and visibility, and a free destination. It pushes only the inspected
current-branch commit, without force or automatic tags, verifies the remote tip,
then moves the checkout. Existing branches and ignored local files move with it.
`--timeout` bounds forge and Git calls (120 seconds by default). Dry-run performs
read-only prerequisite checks without creating, pushing or moving anything.

After a failed push or move, review the reported state and retry the same command
with `--resume`. Resume requires a matching origin and remote visibility. If remote
creation succeeded before origin could be configured, verify the remote and add
that origin manually first. repot never deletes remote or local progress to roll
back a partial publication.

Scratch creation and relocation refuse all existing destination paths, including
symlinks and concurrent collisions. Moves are atomic within one filesystem;
checkouts with linked worktrees, submodules, borrowed object databases, symlinked Git metadata or separate
worktree configuration require manual handling or registration in place.

## Find, adopt and restore

```sh
repot find ~/Projects --json
repot adopt ~/Downloads/project --dry-run
repot adopt ~/Downloads/project
repot adopt ~/.local/share/chezmoi --register
repot restore --dry-run
repot restore --timeout 120 --json
```

`find` skips known checkouts, symlinks, generated directories and common caches.
`adopt` moves a standalone checkout to its remote's tree location and registers it;
`--register` leaves it in place. Dirty files are preserved when moving. A checkout
without a remote can be registered in place with restoration disabled.

The TOML manifest is portable and editable by hand:

```toml
[settings]
owners = ["your-name"]

[[repo]]
url = "https://github.com/your-name/project"

[[repo]]
url = "https://github.com/your-name/dotfiles"
path = "~/.local/share/chezmoi"
restore = false
```

Paths expand `~` and environment variables; `$$` represents a literal dollar.
Relative paths resolve against the manifest's directory. Registration uses home-relative
paths where possible. Comment-preserving updates follow a symlinked manifest to its
dotfiles source, use a persistent companion `.lock` file, and serialize concurrent
registrations. `adopt --manifest PATH` can create a new manifest.

Restore skips `restore = false` entries and **every** existing destination, including
files and dangling symlinks. Missing checkouts are cloned into private staging
folders and installed with an atomic no-overwrite rename. One clone failure does
not prevent independent entries from restoring. Submodules are not cloned
recursively. Local absolute/file remotes are supported only for entries with an
explicit destination path; credential-bearing URLs and shell helper protocols are
rejected.

## Development

Install [mise](https://mise.jdx.dev/), then from the checkout:

```sh
mise install            # pinned Rust toolchain, nextest, bacon, watchexec, dagger
mise run ci-native      # formatting, type check, strict Clippy and tests on the host
mise run ci             # the same checks in Linux through Dagger (needs a container engine)
mise run build          # optimized binary in target/release/repot
mise run package        # native tarball and checksum in dist/
mise run nix-check      # isolated source build and tests with locked Nix inputs
mise run run -- --help  # run the CLI from source
```

`mise run dev` keeps Clippy feedback open with bacon, and `mise run watch` reruns checks and tests on every change. `mise tasks` lists everything.

On macOS, `mise run ci` needs a Docker-compatible engine such as [Colima](https://github.com/abiosoft/colima) (`colima start`).

Runtime dependencies are scoped to concrete needs: `serde`/`serde_json` for reports,
`toml`/`toml_edit` for validated, comment-preserving manifests, `tempfile` for staging,
`nix` for Unix process-group cancellation, and `rustix` for atomic no-overwrite moves.
The existing `clap` dependency handles the command interface.
`ratatui`, `crossterm`, `nucleo-matcher` and `signal-hook` provide the embedded
picker and terminal cleanup. `gix` reads supported Git metadata in-process; Git
remains authoritative for status and mutations. `portable-pty` is a test-only
dependency for actual terminal and shell workflows.
`ureq` with Rustls and `html5gum` implement bounded Go vanity metadata resolution;
`clap_complete` and `clap_complete_nushell` generate shell completions.

Tests invoke the real binary with temporary homes, checkouts and local bare remotes.
Forge responses and failures are controlled by local shims; no test publishes to a
real forge. Four deterministic seeds (`7`, `42`, `2026`, `65537`) run 64 persistent
Git transitions, checking dry-run immutability, commit ancestry, local-file/index
preservation and failure recovery. Failures include the seed and action trace.
Shell tests exercise supported installed shells; Nix checks supply all four.
The picker also has deterministic input-state simulations and PTY tests for
selection, cancellation, resizing, terminal restoration and shell handoffs.

## Performance

On one macOS ARM64 machine with 24 small local repositories, nine warm-cache
measurements produced these medians. Listing uses `list -p`; status uses
`status --no-fetch --json --jobs 4`.

| Root selection | repot list | ghq list | repot status | Baseline status |
| --- | ---: | ---: | ---: | ---: |
| `GHQ_ROOT` | 3.44 ms | 5.05 ms | 576 ms | 888 ms |
| Git `ghq.root` | 7.15 ms | 12.65 ms | 580 ms | 901 ms |

Both root modes produced matching sorted listings and identical baseline/current
status JSON. The baseline is repot revision `e037a66`; ghq 1.10.1 has no equivalent
full status command. Native metadata reads replace repeated Git subprocesses for
operation-marker lookups, while other changes also affect full-command timings.

These local synthetic measurements cover process startup and cached filesystem
work, without network cloning. [Methodology and raw samples](docs/benchmarks/README.md)
include executable hashes, a reproducible script and an isolated native/Git
microbenchmark.
