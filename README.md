<p align="center"><img src="assets/repot.png" width="112" alt="repot: a Git branch sprouting from a terracotta pot"></p>

<h1 align="center">repot</h1>

<p align="center">Keep every Git repository on your machine organised, current and portable.</p>

<p align="center">
  <a href="https://github.com/niklas-heer/repot/releases/latest"><img src="https://img.shields.io/github/v/release/niklas-heer/repot" alt="Latest release"></a>
  <a href="https://github.com/niklas-heer/repot/actions/workflows/ci.yml"><img src="https://github.com/niklas-heer/repot/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/niklas-heer/repot" alt="MIT license"></a>
</p>

<p align="center"><a href="#install">Install</a> · <a href="#quick-start">Quick start</a> · <a href="docs/guide.md">User guide</a> · <a href="docs/configuration.md">Configuration</a></p>

---

repot gives your repositories a home: `<root>/<host>/<owner>/<repo>`. Clone a
project, jump to it with the built-in fuzzy picker, and safely bring your
checkouts up to date. Start an experiment locally, publish it when it's ready,
and restore the projects you care about on your next machine.

Already use [ghq](https://github.com/x-motemen/ghq)? Your existing tree and Git
configuration work as they are. Neither ghq nor fzf is required.

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
brew tap niklas-heer/repot https://github.com/niklas-heer/repot
brew install niklas-heer/repot/repot
```

### Nix

```sh
nix profile install github:niklas-heer/repot/v0.1.0
```

Prefer a binary or a source build? See the [release downloads](https://github.com/niklas-heer/repot/releases/tag/v0.1.0)
and [installation guide](docs/install.md) for platform selection, checksums and Cargo instructions.

### Enable directory switching

Add the line for your shell to its configuration, then reload it:

| Shell | Configuration |
| --- | --- |
| Bash | `eval "$(repot shell-init bash)"` in `~/.bashrc` |
| Zsh | `eval "$(repot shell-init zsh)"` in `~/.zshrc` |
| Fish | `repot shell-init fish \| source` in `~/.config/fish/config.fish` |
| Nushell | Save `repot shell-init nu` to a file and source it from `config.nu`; [instructions](docs/install.md#nushell). |

The wrapper lets `jump`, `new` and other relocation commands change your shell's
directory. Without it, repot prints the selected path. Completions are available
for all four shells with `repot completions <shell>`.

## Quick start

```sh
repot get niklas-heer/repot     # clone into your repository tree
repot jump repot               # enter the checkout
repot status                  # fetch and see what needs attention
repot sync --dry-run           # preview using cached remote refs
repot sync                    # fetch and apply safe updates
```

The default root is `~/ghq`. Existing `GHQ_ROOT` and Git `ghq.root` settings take
precedence. Run `repot root` to see yours.

## Everyday workflows

### Find a project and get back to work

```sh
repot jump                    # open the fuzzy picker
repot jump project            # jump directly when the match is unique
repot list -p                 # print full paths for scripts
```

Type to filter; use arrows or <kbd>Ctrl</kbd>+<kbd>N</kbd>/<kbd>P</kbd> to select,
<kbd>Enter</kbd> to jump, and <kbd>Esc</kbd> to cancel. The picker includes registered
repositories outside the tree, such as your dotfiles.

### Keep your checkouts current

```sh
repot status --json
repot sync --jobs 4 --timeout 30
```

Sync fast-forwards clean branches and returns safely merged feature branches to
the remote's default branch when it can prove their work is preserved. Squash and
rebase merges can be recognized through patch equivalence or an exact merged
GitHub PR match with optional `gh`. Ambiguous cases stay for review. Sync never
pushes or deletes your branches. [How safety checks work →](docs/guide.md#status-and-sync)

### Start scratch, publish later

```sh
repot new experiment          # create locally, with no remote or commit
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
repot find ~/Projects
repot adopt ~/Downloads/project --dry-run
repot adopt ~/.local/share/chezmoi --register   # keep this checkout in place
repot restore --dry-run
repot restore                                # clone missing manifest entries
```

Restore leaves every existing destination untouched. Keep the manifest in your
dotfiles to bring the same set of projects to another machine.

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
