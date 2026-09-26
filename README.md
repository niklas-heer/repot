# repot

Keep every Git repository on a machine organised, current and portable.

repot builds on the [ghq](https://github.com/x-motemen/ghq) directory tree (`<root>/<host>/<owner>/<repo>`). It adds what ghq leaves out: jumping to any repository with a fuzzy picker, safely updating everything at once (including returning to `main` after a branch was merged on GitHub), scratch projects that can be published into the tree later, finding stray repositories, and restoring a machine from a manifest kept in your dotfiles.

> **Status:** early development. Nothing beyond `--version` and `--help` works yet. See [BUILD_BRIEF.md](BUILD_BRIEF.md) for the planned scope, safety rules and milestones.

## Planned commands

| Command | Purpose |
| --- | --- |
| `repot jump` | Fuzzy-pick a repository and `cd` into it |
| `repot status` / `repot sync` | See every repository's state; fast-forward what is safe |
| `repot new` / `repot publish` | Start a local scratch project; later create its remote and move it into the tree |
| `repot find` / `repot adopt` | Discover repositories outside the tree and bring them in |
| `repot restore` | Clone everything from your manifest on a new machine |

repot never commits, stashes, resets or force-pushes, and only updates a repository by fast-forward.

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
