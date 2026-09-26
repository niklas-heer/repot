# repot

Keep every Git repository on a machine organised, current and portable.

repot builds on the [ghq](https://github.com/x-motemen/ghq) directory tree (`<root>/<host>/<owner>/<repo>`). It adds what ghq leaves out: jumping to any repository with a fuzzy picker, safely updating everything at once (including returning to `main` after a branch was merged on GitHub), scratch projects that can be published into the tree later, finding stray repositories, and restoring a machine from a manifest kept in your dotfiles.

> **Status:** milestone 1 is implemented: `list`, `jump`, and `shell-init`. The remaining commands below are planned. See [BUILD_BRIEF.md](BUILD_BRIEF.md) for safety rules and milestones.

## Commands (remaining milestones planned)

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
