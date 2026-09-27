# User guide

[← Overview](../README.md) · [Configuration](configuration.md) · [Installation](install.md)

## Command reference

| Command | Purpose |
| --- | --- |
| `repot cd [query]` | Jump to a repository; an inline fuzzy picker opens when several match |
| `repot info [query]` | Branch, sync state, local changes, remote, visits and recent commits |
| `repot list [query]` | List discovered and registered repositories (`ls` also works) |
| `repot root [--all]` | Show primary or all configured roots |
| `repot status` / `repot sync` | See what needs attention; fast-forward what is safe |
| `repot clone <repo>…` | Clone one or many repositories; safely update with `-u` (`get` also works) |
| `repot new <name>` | Start a local scratch project; `owner/name` creates an empty repository in the tree |
| `repot publish <owner/name>` | Create the remote for a scratch project, push, and move it into the tree |
| `repot scan [path]` / `repot adopt <path>` | Discover repositories outside the tree and bring them in |
| `repot restore` | Clone missing entries from your manifest on a new machine |
| `repot rm` / `repot trash` | Archive a checkout without losing work; list or restore archives |
| `repot doctor` | Find duplicates, renamed or archived GitHub repositories and misplaced checkouts |
| `repot shell-init <shell>` | Print shell integration so `repot cd` can change directory |
| `repot completions <shell>` | Generate bash, zsh, fish or Nushell completions |
| `repot agent-guide` / `repot mcp` | Guide and stdio MCP server for coding agents |

repot never commits, stashes, resets or force-pushes, and only updates a repository by fast-forward.

### Names and compatibility

Commands are named for what you want to do, so they read naturally to someone
new: `cd` to go somewhere, `clone` to bring a repository in, `new` to start one,
`scan` to look for strays. The ghq-style names keep working for muscle memory
and scripts: `jump` runs `cd`, `get` runs `clone`, `find` runs `scan`, and the
hidden `create` and `migrate` commands behave like `new owner/name` and `adopt`.

## Navigation

Root settings and manifest selection are described in [Configuration](configuration.md).

```sh
repot list --json
repot list -p --exact repot
repot cd repot
# Add the appropriate integration to your shell configuration:
eval "$(repot shell-init bash)"    # use zsh in zsh
repot shell-init fish | source    # fish
# Nushell: save `repot shell-init nu` to a file and source that file.
# Optional completions (save/source using your shell's normal convention):
repot completions nu
```

The embedded picker opens inline, directly under your prompt, like `fzf --height`.
It never takes over the screen, so the output you were just reading stays in view,
and it erases itself when you are done. Type to filter, use arrows or Ctrl+N/P to
choose, Enter to go, Escape or Ctrl+C to cancel, and Ctrl+U to clear the query.
Matches are highlighted and the repository name stands out from its owner. It
handles resizing, Ctrl+Z and `NO_COLOR`. Exact and unique matches jump directly;
noninteractive scripts must provide an unambiguous query. Without the shell
wrapper, `cd` prints the selected path, so `cd "$(repot cd query)"` also works. Directory symlinks are not traversed;
linked worktrees can be discovered or registered, and nested submodules are not
listed separately.

### Ranking: where you actually go

With an empty query, the picker lists the repositories you use first. Two
signals decide the order:

- **Frecency.** Every switch into a repository is logged, and recent visits weigh
  more than old ones: a visit in the last hour counts 16, today 8, this week 4,
  this month 2, and anything older 1. A daily habit therefore beats a single
  visit, while the repository you just left stays near the top.
- **Git activity.** Repositories you have not visited yet are ordered by their
  latest local Git activity (commits, checkouts, merges and staging).

While you type, fuzzy matching decides and frecency breaks close calls; it never
adds a repository that does not match.

The shell integration records visits whenever your shell enters a checkout,
however you got there, and `repot cd` records its own selections. Moving between
directories of the same checkout is not a new visit. The log is a small text
file at `$XDG_STATE_HOME/repot/visits` (usually `~/.local/state/repot/visits`)
holding only timestamps and checkout paths; it stays on your machine and is
trimmed to the latest 4,000 visits. Generate the integration with `--no-track`,
for example `repot shell-init zsh --no-track`, to switch tracking off, and delete
the file to forget your history.

### Repository details

`repot info` shows the checkout you are in, `repot info NAME` any other one, and
`repot info` outside a checkout opens the picker. It prints the path, remote,
branch with ahead/behind counts from cached remote refs, staged, modified,
untracked and stashed work, your visits and the five latest commits. `--json`
gives the same data to scripts. Remote URLs never show embedded credentials.

In terminals at least 100 columns wide, the picker shows the same details for
the selected repository beside the results; they load in the background, so
typing never waits for Git.

`list` prints root-relative paths by default, `-p` prints absolute paths, `-e`
matches exact suffixes, and `--unique` prints the shortest unambiguous names.
JSON retains absolute paths. Bare repositories appear in `list`; bulk status and
sync operate on working checkouts. `root --all` prints roots in priority order.

## Cloning, importing and archives

Existing checkouts stay where they are. Use repot for the same Git tasks:

```sh
repot clone owner/project
repot clone -p github.com/owner/project     # SSH
repot clone --shallow --branch main owner/project
repot clone --partial blobless owner/large-project
repot clone --bare owner/project
repot clone --update owner/project         # fast-forward only
repot list | repot clone --parallel        # newline-separated import
repot clone --look owner/project           # cd through shell-init
repot new owner/new-project                # init only; no remote or commit
repot adopt ~/Downloads/project --dry-run
```

`clone` also accepts multiple arguments, `repo@branch`, `--partial treeless`,
`--silent`, `--no-recursive`, and `--vcs git` (or `github` / `codecommit`). Short repository
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

## Status and sync

```sh
repot status --json
repot sync --dry-run
repot sync --jobs 24 --timeout 30
```

Status fetches the relevant remote for each checkout with bounded parallelism
(24 at a time by default) and shows a live counter while it works. Each checkout
needs a single connection: the remote's default branch is read from the cached
`origin/HEAD` when the checkout is already on it. Sync updates each checkout as
soon as its own inspection finishes, while other fetches are still running.

Fetching is dominated by connection setup, especially over SSH. If status still
feels slow, SSH connection sharing lets fetches reuse one authenticated
connection per host:

```sshconfig
Host github.com
  ControlMaster auto
  ControlPath ~/.ssh/control-%C
  ControlPersist 60
``` In a
terminal, the report groups repositories by what they need: **Failed**, **Held
back by local changes** (behind, but uncommitted work blocks a fast-forward),
**Needs review**, **Ready to push**, **Ready to update** (or **Updated** after a
sync), **Work in progress** (nothing new upstream, only your own uncommitted work
or stashes), and a compact **Up to date** summary. Each row shows the branch,
commits behind (↓) or ahead (↑), and staged, modified, untracked and stashed work,
followed by a concrete suggestion such as `git push -u origin feature` for an
unpushed branch or `repot publish` for a checkout without a remote. The grouping
is presentation only: JSON `action` values and exit codes are unchanged. The footer suggests the next step, such as
`repot sync`. Piped output keeps one tab-separated line per repository for
scripts, and `--json` stays the stable machine interface.
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
repot new owner/project               # empty repository at <root>/github.com/owner/project
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

## Find duplicates and renamed repositories

Renaming or transferring a repository on GitHub leaves the old checkout behind,
and cloning the new name then gives you two copies. `repot doctor` finds that
and other drift, and prints the command that fixes each finding. It changes
nothing itself.

```sh
repot doctor              # asks GitHub through gh when it is installed
repot doctor --offline    # local checks only
repot doctor --json
```

| Finding | Meaning | Suggested fix |
| --- | --- | --- |
| Duplicate | Two checkouts of the same repository, also across a rename | `repot rm` the stale copy |
| Renamed on GitHub | The remote still uses an old name or owner | `git remote set-url`, then `repot adopt` to move it |
| Not where its remote belongs | The folder no longer matches `host/owner/name` | `repot adopt` |
| Not found on GitHub | Deleted, or not visible to your `gh` account | `repot info` to review |
| Archived on GitHub | Read-only upstream | `repot rm` when you no longer need it |

For duplicates, the checkout already at its current name's location is kept.
Before suggesting removal, doctor lists work that exists only in the stale copy:
changed files, stashes, unpushed commits and linked worktrees. `repot rm`
archives the entire checkout, so `repot trash restore` can bring it back. Doctor
exits 3 when it has findings and 0 when everything is where it belongs.

## Scan, adopt and restore

```sh
repot scan ~/Projects --json
repot adopt ~/Downloads/project --dry-run
repot adopt ~/Downloads/project
repot adopt ~/.local/share/chezmoi --register
repot restore --dry-run
repot restore --timeout 120 --json
```

`scan` skips known checkouts, symlinks, generated directories and common caches.
`adopt` moves a standalone checkout to its remote's tree location and registers it;
`--register` leaves it in place. Dirty files are preserved when moving. A checkout
without a remote can be registered in place with restoration disabled.

See [Configuration](configuration.md) for TOML, KDL and YAML manifests, path expansion,
comment-preserving registration and dotfiles symlinks.

Restore skips `restore = false` entries and **every** existing destination, including
files and dangling symlinks. Missing checkouts are cloned into private staging
folders and installed with an atomic no-overwrite rename. One clone failure does
not prevent independent entries from restoring. Submodules are not cloned
recursively. Local absolute/file remotes are supported only for entries with an
explicit destination path; credential-bearing URLs and shell helper protocols are
rejected.
