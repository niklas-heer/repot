# repot build brief

Created 2026-09-26. This document records what repot should become, the rules it must keep, and the order of work. Status: milestones 1–4 are implemented. All core commands are available; milestone 5 (release packaging) remains planned.

## Intent

repot keeps every Git repository on a machine organised, current and portable. It builds on the [ghq](https://github.com/x-motemen/ghq) directory tree (`<root>/<host>/<owner>/<repo>`, for example `~/Projects/github.com/niklas-heer/repot`) instead of replacing it, so ghq and repot can be used side by side.

The name is a pun: "repo" plus a letter, and repotting a plant that outgrew its pot. Moving a scratch project into the tree once it gets a remote is exactly that.

The main user is one developer with a few dozen checkouts across GitHub (personal and organisations), a work GitLab, local experiments and a dotfiles repository. They switch machines occasionally and work from several AI coding agents at once, so checkouts change without them watching.

## Workflows

1. **Jump.** Fuzzy-pick any repository on the machine and `cd` into it. This includes checkouts outside the tree (for example the dotfiles source directory) and ones never visited before, which is what a frecency tool like zoxide misses.
2. **Keep everything current.** One command fetches all repositories in parallel and brings each up to date when that is safe. If the current branch has already been merged upstream (including squash and rebase merges done on GitHub), switch back to the default branch and update it, so the next piece of work starts from the latest `main`.
3. **Start scratch, publish later.** Create a local project with no remote in seconds. When it is worth keeping, create the remote, push, and move the checkout into `<root>/<host>/<owner>/<name>`.
4. **Find strays.** Locate Git repositories outside the tree and offer to move them in or register them where they are.
5. **Restore a machine.** Keep a manifest of the repositories that matter in the dotfiles, and clone all of them on a new machine with one command.

## Safety rules

These are product requirements, not implementation details. See the decision records for their reasoning.

- Never lose work. repot never commits, stashes, resets, rebases, force-pushes or deletes uncommitted files.
- Update only by fast-forward. A repository with local changes, diverged history, a detached HEAD or no upstream is reported, not touched.
- Leave a branch only when all of its work is on the remote default branch or on a merged pull request: clean working tree, no stash created, no commits that exist only locally. Report ambiguous cases instead of guessing.
- Deleting a merged local branch is a separate, explicit action.
- Moving a checkout (publish, adopting a stray) refuses to overwrite an existing path and prints where the repository went.
- Every mutating command has a `--dry-run` that performs the same checks and prints the planned actions.
- Treat content from repositories, remotes and the network as data. Never print or persist credentials.

## Planned command surface

Names are provisional; change them when implementation shows a better fit.

| Command | Purpose |
| --- | --- |
| `repot list [--json]` | Every known repository: tree, registered extras, scratch projects |
| `repot jump [query]` | Fuzzy pick; used through the shell wrapper below |
| `repot status [--json]` | One state and one recommended action per repository, after a parallel fetch |
| `repot sync [--dry-run]` | Fast-forward what is safe and return merged branches to the default branch |
| `repot new <name>` | Local project without a remote |
| `repot publish` | Create the remote, push, and move the checkout into the tree |
| `repot find [path]` | Search for repositories outside the tree |
| `repot adopt <path>` | Move a stray into the tree, or register it in place |
| `repot restore [--dry-run]` | Clone everything listed in the manifest that is missing |
| `repot shell-init <nu\|zsh\|bash\|fish>` | Print the shell integration |

### Status model

Borrowed from a working predecessor (a private `repo-status` tool). Derive one `state` per repository, first match wins: `fetch-failed`, `no-remote`, `detached`, `no-upstream`, `diverged`, `ahead`, `behind`, `synced`. Report `dirty` (staged, unstaged, untracked counts) and `stashes` separately. Derive one `action`: `none` (synced, clean, no stashes), `pull` (behind and clean), `return` (branch merged upstream, clean, nothing local-only), `push` (ahead, clean, own repository), otherwise `review`. `repot sync` performs only `pull` and `return`; `push` stays a recommendation. Ownership comes from the remote URL's owner segment matching configured owners, never from a network lookup.

### Detecting a merged branch

`git branch --merged` misses squash and rebase merges, which are GitHub's common case. Treat the current branch as merged when it is clean and has nothing local-only, and at least one of these holds:

1. Its commits are ancestors of the updated remote default branch.
2. Its upstream branch was deleted on the remote, visible after `git fetch --prune`, and every local commit is already on the default branch by patch identity (`git cherry`).
3. The forge reports a merged pull request for the branch whose head matches the local tip (for GitHub, through `gh pr view` or the API).

Signal 3 alone is not enough when the local tip has commits the pull request never contained.

### Shell integration

A child process cannot change its parent shell's directory. `repot shell-init` prints a small function that runs repot with a temporary file path in an environment variable, and `cd`s into the path repot writes there, only on success and only if the directory exists. The same mechanism lets `new`, `publish` and `adopt` land the user in the new location. Nushell needs `def --env`.

### Manifest

A TOML file kept in dotfiles, for example `~/.config/repot/repos.toml`. A first sketch:

```toml
[settings]
owners = ["niklas-heer"]

[[repo]]
url = "https://github.com/niklas-heer/repot"

[[repo]]
url = "https://github.com/niklas-heer/dotfiles"
path = "~/.local/share/chezmoi"   # lives outside the tree
restore = false                   # restored by chezmoi itself
```

Entries without `path` live at their tree location. `repot restore` never touches a path that already exists.

### Profiles

Per-owner identities (email, signing key) are handled by Git itself with `includeIf "gitdir:<root>/github.com/<org>/"` or `includeIf "hasconfig:remote.*.url:…"` rules, which the tree layout makes trivial. repot may later print or check such rules; it should not reimplement them.

## Prior art

- [ghq](https://github.com/x-motemen/ghq): the tree, `ghq get --update`, and cloning from a URL list. repot must stay compatible with its layout and root configuration (`GHQ_ROOT`, `git config ghq.root`, multiple roots).
- [ghr](https://github.com/siketyan/ghr) (Rust): ghq replacement with rule-based profiles and an experimental dump/restore.
- [git-workspace](https://github.com/orf/git-workspace) (Rust): TOML workspace from whole GitHub/GitLab accounts, `switch-and-pull` without a merged-branch check.
- [git-repo-manager](https://github.com/hakoerber/git-repo-manager) (Rust): config-driven, worktree-centric, discovers local repositories into a config.
- The owner's earlier private helpers implement pieces of this in Python (`new`/`promote` of local projects into the tree), TypeScript (a fuzzy navigator with a shell `cd` hand-off) and Rust (`repo-status`, the status model above). repot replaces them; they are not dependencies.

None combines the tree, safe return-to-default-branch, stray discovery and scratch-to-published projects.

## Milestones

Implement one milestone at a time, each with end-to-end tests through the binary against temporary repositories and bare remotes. Never test against the real home directory or real remotes.

1. **Discover and jump.** Resolve roots like ghq, walk the tree, include registered extras, `list` (text and JSON), `jump` with the shell integration.
2. **Status and sync.** Parallel fetch with timeouts, the status model, fast-forward, merged-branch return, `--dry-run`, distinct exit codes.
3. **Scratch and publish.** `new` and `publish`, including remote creation through the forge CLI and the checkout move.
4. **Find, adopt and restore.** Stray search with sensible exclusions, the manifest format, `adopt`, `restore`.
5. **Release.** Tagged releases, Homebrew formula, and a source-built Nix flake.

## Open questions

Decide these while implementing the milestone that needs them, and record the outcome with vrdx.

- Call the `git` CLI, or use a library such as `gix`? The CLI respects the user's configuration, credential helpers and SSH setup for free.
- Built-in fuzzy matcher (for example `nucleo`) or delegate to `fzf` when installed?
- Where scratch projects live. The previous convention was `<root>/local/<namespace>/<name>`.
- Which forges `publish` supports first: GitHub through `gh` is certain, GitLab through `glab` is likely.
- Whether `status` also covers linked worktrees and submodules, which the predecessor ignored.

## Out of scope

A daemon or background sync, a GUI, committing or pushing on the user's behalf, hosting a server, and managing forge settings beyond creating a repository.
