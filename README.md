<p align="center"><img src="assets/repot.png" width="112" alt="repot: a Git branch sprouting from a terracotta pot"></p>

<h1 align="center">repot</h1>

<p align="center">Keep every Git repository on your machine organised, current and portable.</p>

<p align="center">
  <a href="https://github.com/niklas-heer/repot/releases/latest"><img src="https://img.shields.io/github/v/release/niklas-heer/repot" alt="Latest release"></a>
  <a href="https://github.com/niklas-heer/repot/actions/workflows/ci.yml"><img src="https://github.com/niklas-heer/repot/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/niklas-heer/repot" alt="MIT license"></a>
</p>

<p align="center"><a href="#install">Install</a> · <a href="#quick-start">Quick start</a> · <a href="#repot-and-ghq">repot and ghq</a> · <a href="docs/guide.md">User guide</a> · <a href="docs/configuration.md">Configuration</a></p>

<p align="center"><img src="demo/repot.gif" width="860" alt="repot status groups ten checkouts by what they need, repot sync fast-forwards four of them and returns a squash-merged branch to main, the inline picker previews and jumps to a repository, and repot clone brings in a new one"></p>

---

You have dozens of checkouts, and several coding agents working in them. Which
ones are behind? Which branch was merged last week? Where did that prototype go?
repot gives every repository a home, `<root>/<host>/<owner>/<repo>`, and keeps
the whole tree current without ever risking your work.

- **Jump anywhere.** `repot cd` opens a fuzzy picker right under your prompt,
  ranked by how often and how recently you went there, with a live preview.
- **See everything at once.** `repot status` fetches all checkouts in parallel
  and groups them by what they need: update, push or review.
- **Update safely.** `repot sync` fast-forwards what is clean and returns merged
  branches to `main`, including squash and rebase merges.
- **Start scratch, publish later.** `repot new` starts locally; `repot publish`
  creates the GitHub or GitLab remote, pushes and moves it into the tree.
- **Take it with you.** A TOML, KDL or YAML manifest restores the same projects
  on your next machine.

Already use [ghq](https://github.com/x-motemen/ghq)? Your existing tree and Git
configuration work as they are. Neither ghq nor fzf is required.
[How they compare →](#repot-and-ghq)

**Your work stays yours.** Updates are fast-forward only. Dirty or diverged
checkouts need review. repot never commits, stashes, resets, rebases or force-pushes,
and removing a checkout archives it for recovery. Every mutating command has
`--dry-run`.

## Install

Linux and macOS, on x86-64 and ARM64. **Git is required** for repository operations;
Homebrew and Nix supply it. Install [`gh`](https://cli.github.com/) or
[`glab`](https://gitlab.com/gitlab-org/cli) when you want to publish projects.

### Homebrew

```sh
brew install niklas-heer/tap/repot
```

The formula lives in [niklas-heer/homebrew-tap](https://github.com/niklas-heer/homebrew-tap)
alongside the other tools. Tapped the old `niklas-heer/repot` location? `brew update`
moves the installation over automatically.

### Nix

```sh
nix profile install github:niklas-heer/repot/v0.3.0
```

Prefer a binary or a source build? See the [release downloads](https://github.com/niklas-heer/repot/releases/tag/v0.3.0)
and [installation guide](docs/install.md) for platform selection, checksums and Cargo instructions.

### Enable directory switching

Add the line for your shell to its configuration, then reload it:

| Shell | Configuration |
| --- | --- |
| Bash | `eval "$(repot shell-init bash)"` in `~/.bashrc` |
| Zsh | `eval "$(repot shell-init zsh)"` in `~/.zshrc` |
| Fish | `repot shell-init fish \| source` in `~/.config/fish/config.fish` |
| Nushell | Save `repot shell-init nu` to a file and source it from `config.nu`; [instructions](docs/install.md#nushell). |

The wrapper lets `cd`, `new` and other relocation commands change your shell's
directory. Without it, repot prints the selected path. Completions are available
for all four shells with `repot completions <shell>`.

## Quick start

```sh
repot clone niklas-heer/repot   # clone into your repository tree
repot cd repot                 # enter the checkout
repot status                   # fetch and see what needs attention
repot sync --dry-run           # preview using cached remote refs
repot sync                     # fetch and apply safe updates
```

The default root is `~/ghq`. Existing `GHQ_ROOT` and Git `ghq.root` settings take
precedence. Run `repot root` to see yours.

## Everyday workflows

### Find a project and get back to work

```sh
repot cd                      # open the inline fuzzy picker
repot cd project              # jump directly when the match is unique
repot list -p                 # print full paths for scripts
```

The picker opens right under your prompt instead of taking over the screen, so
whatever you were looking at stays visible. It puts the repositories you use most
and most recently at the top, and on wide terminals previews the selected
repository's branch, changes and recent commits. `repot info` prints the same
details for the checkout you are in, and `repot open` (or `repot open --web`)
opens it in your editor or on GitHub. Type to filter; use arrows or
<kbd>Ctrl</kbd>+<kbd>N</kbd>/<kbd>P</kbd> to select, <kbd>Enter</kbd> to go, and
<kbd>Esc</kbd> to cancel. It includes registered repositories outside the tree,
such as your dotfiles.

### Keep your checkouts current

```sh
repot status                  # grouped report with live progress
repot status --json           # stable output for scripts
repot sync --jobs 24 --timeout 30
```

`status` groups checkouts by what they need (review, push, update) and
summarises the healthy ones, then suggests the next step.

Sync fast-forwards clean branches and returns safely merged feature branches to
the remote's default branch when it can prove their work is preserved. Squash and
rebase merges can be recognized through patch equivalence or an exact merged
GitHub PR match with optional `gh`. Ambiguous cases stay for review. Sync never
pushes or deletes your branches. [How safety checks work →](docs/guide.md#status-and-sync)

### Start scratch, publish later

```sh
repot new experiment          # create locally, with no remote or commit
repot new api --template owner/starter   # or start from a template's files
# Work and commit as usual, then preview and publish:
repot publish your-name/experiment --visibility public --dry-run
repot publish your-name/experiment --visibility public
```

Publishing creates the remote, pushes the inspected commit, verifies the result,
and moves the checkout into the tree. GitHub and GitLab are supported. Partial
progress is preserved; the [publishing guide](docs/guide.md#scratch-projects-and-publishing)
explains prerequisites and `--resume`.

### Bring in strays or restore a machine

```sh
repot scan ~/Projects
repot adopt ~/Downloads/project --dry-run
repot adopt ~/.local/share/chezmoi --register   # keep this checkout in place
repot restore --dry-run
repot restore                                # clone missing manifest entries
```

Restore leaves every existing destination untouched. Keep the manifest in your
dotfiles to bring the same set of projects to another machine.

### Clean up after renames

```sh
repot doctor                  # duplicates, renamed and misplaced checkouts
```

Renamed a repository on GitHub and cloned the new name? Doctor spots the stale
copy, shows any work that exists only there, and suggests the exact command.

### Remove a checkout without losing its work

```sh
repot rm owner/old-project --dry-run
repot rm owner/old-project
repot trash list
repot trash restore ARCHIVE_ID
```

The archive retains commits, dirty and ignored files, stashes and the index.
Restoration refuses an occupied destination.

## Keep a portable manifest

Choose **TOML, KDL v2 or YAML** in `$XDG_CONFIG_HOME/repot/` (usually `~/.config/repot/`).
repot detects `repos.toml`, `repos.kdl`, `repos.yaml` or `repos.yml`. If more than
one exists, select one with `--manifest PATH`.

<details>
<summary><strong>TOML — repos.toml</strong></summary>

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

</details>

<details>
<summary><strong>KDL v2 — repos.kdl</strong></summary>

```kdl
settings { owners "your-name"; }

repo "https://github.com/your-name/project"
repo "https://github.com/your-name/dotfiles" path="~/.local/share/chezmoi" restore=#false
```

</details>

<details>
<summary><strong>YAML — repos.yaml</strong></summary>

```yaml
settings:
  owners: [your-name]

repo:
  - url: https://github.com/your-name/project
  - url: https://github.com/your-name/dotfiles
    path: ~/.local/share/chezmoi
    restore: false
```

</details>

`path` is optional; `restore` defaults to true. Registration preserves comments
and follows symlinks into your dotfiles. [Configuration reference →](docs/configuration.md)

## Working with agents

Run `repot agent-guide` for an offline guide to noninteractive commands, JSON
reports, exit codes and safe automation. Agents can also connect through the
local stdio MCP server with `repot mcp`. See the [agent guide](docs/agents.md) for
client setup and how to plan, apply and verify changes.

## repot and ghq

repot started as a replacement for [ghq](https://github.com/x-motemen/ghq)'s Git
workflows. It keeps ghq's directory layout, `GHQ_ROOT`, the `ghq.*` Git settings
and its command names (`get`, `create` and `migrate` still work), so it
picks up your existing tree as it is. Where ghq manages clones, repot also looks
after what happens to them afterwards.

| | ghq | repot |
| --- | --- | --- |
| Tree layout and root settings | `<root>/<host>/<owner>/<repo>` | The same tree and settings |
| Jump to a repository | Pipe `ghq list` into fzf or peco | Built-in picker with preview, ranked by use |
| What needs attention? | Not covered | `status`: one parallel fetch, grouped by next step |
| Update checkouts | `ghq get -u` on the repositories you name | `sync`: everything at once, fast-forward only, merged branches return to `main` |
| New projects | `create` initializes one in the tree | `new` starts a scratch project, optionally from a template; `publish` creates the remote and moves it in |
| Stray checkouts | `migrate` moves one you point at | `scan` finds them; `adopt` moves or registers them in place |
| New machine | `ghq list` into a file, then `ghq get` | A manifest and `restore`, including checkouts outside the tree |
| Removing | Deletes the checkout | Archives it with stashes, index and ignored files; `trash restore` |
| Housekeeping | Not covered | `doctor` for duplicates and renames; `stale` for forgotten checkouts |
| Automation | Plain text | `--json`, documented exit codes, an agent guide and an MCP server |
| Version control | Git, Subversion, Mercurial, Darcs, Fossil, Bazaar | Git only |
| Platforms | Linux, macOS, Windows | Linux and macOS |

**Choose ghq** if you need Subversion, Mercurial or another non-Git backend, work
on Windows, or prefer a tool that does one job and leaves the rest to your own
scripts. It is mature, widely packaged and has more than a decade of use
behind it.

**Choose repot** if you want the whole loop in one binary: find, update, start,
publish, clean up and restore, with safety checks between you and a lost commit.
Listing is also a little faster than ghq's in the
[reproducible benchmarks](docs/benchmarks/README.md).

Both can live side by side on the same tree, so trying repot costs nothing.

## Explore further

| Guide | What you'll find |
| --- | --- |
| [User guide](docs/guide.md) | All commands, clone options, supported URLs, safety rules and exit codes |
| [Configuration](docs/configuration.md) | Multiple roots, TOML/KDL/YAML schemas, path expansion and ghq settings |
| [Agent guide](docs/agents.md) | Noninteractive workflows, JSON reports, exit codes and automation boundaries |
| [Development](docs/development.md) | Pinned tools, native/Dagger/Nix checks, deterministic simulations and PTY tests |
| [Benchmarks](docs/benchmarks/README.md) | Reproducible ghq comparisons, native Git experiments and raw measurements |
| [Design decisions](https://github.com/niklas-heer/repot/tree/main/decisions) | The reasoning behind the implementation |

## License

[MIT](LICENSE).
