# repot

Keep every Git repository on a machine organised, current and portable.

repot builds on the [ghq](https://github.com/x-motemen/ghq) directory tree (`<root>/<host>/<owner>/<repo>`). It adds what ghq leaves out: jumping to any repository with a fuzzy picker, safely updating everything at once (including returning to `main` after a branch was merged on GitHub), scratch projects that can be published into the tree later, finding stray repositories, and restoring a machine from a manifest kept in your dotfiles.

> **Status:** milestones 1–4 are implemented. All core commands are available; release packaging remains planned. See [BUILD_BRIEF.md](BUILD_BRIEF.md) for safety rules and milestones.

## Commands

| Command | Purpose |
| --- | --- |
| `repot jump` | Fuzzy-pick a repository and `cd` into it |
| `repot status` / `repot sync` | See every repository's state; fast-forward what is safe |
| `repot new` / `repot publish` | Start a local scratch project; later create its remote and move it into the tree |
| `repot find` / `repot adopt` | Discover repositories outside the tree and bring them in |
| `repot restore` | Clone everything from your manifest on a new machine |

repot never commits, stashes, resets or force-pushes, and only updates a repository by fast-forward.

## Discovery and navigation

Git is required. Roots follow ghq: `GHQ_ROOT`, then all Git `ghq.root` values,
then `~/ghq`. The final configured root receives new checkouts. Registered manifest
paths are included too. The manifest defaults to `$XDG_CONFIG_HOME/repot/repos.toml`
(or `~/.config/repot/repos.toml`); override it with `--manifest PATH`.

```sh
repot list --json
repot jump repot
# Add the appropriate integration to your shell configuration:
eval "$(repot shell-init bash)"    # use zsh in zsh
repot shell-init fish | source    # fish
# Nushell: save `repot shell-init nu` to a file and source that file.
```

An exact or unique fuzzy match works without extra tools. Ambiguous interactive
selection requires `fzf`; scripts must provide an unambiguous query. Without the
shell wrapper, `jump` prints the selected path. Directory symlinks are not traversed;
linked worktrees can be discovered or registered, and nested submodules are not
listed separately.

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
mise run run -- --help  # run the CLI from source
```

`mise run dev` keeps Clippy feedback open with bacon, and `mise run watch` reruns checks and tests on every change. `mise tasks` lists everything.

On macOS, `mise run ci` needs a Docker-compatible engine such as [Colima](https://github.com/abiosoft/colima) (`colima start`).

Lasting technical choices are recorded in [decisions/](decisions/) with [vrdx](https://github.com/niklas-heer/vrdx).

## License

[MIT](LICENSE)
