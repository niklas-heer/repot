# Configuration

[← Overview](../README.md) · [User guide](guide.md)

## Repository roots

repot uses the existing ghq layout and root configuration. There is no migration
step for existing checkouts.

| Setting | Behavior |
| --- | --- |
| `GHQ_ROOT` | Overrides Git root settings. On Unix, accepts a colon-separated list; the first root is primary. |
| Git `ghq.root` | May contain multiple values; the last value is primary. |
| No root setting | Uses `~/ghq`. |
| Git `ghq.<url>.root` | Selects a root for matching repository URLs. |

```sh
git config --global ghq.root ~/Projects
repot root
repot root --all
```

Existing checkouts are reused across roots. Registered manifest paths are also
discoverable, even outside the tree. `root --all` prints roots in priority order.
Directory symlinks are not traversed; linked worktrees are supported by discovery.
Bare repositories appear in `list`, while bulk status/sync use working checkouts.

Short repository names honor `ghq.user`, `github.user`, `ghq.completeUser`, and
`ghq.defaultHost`. URL-scoped `ghq.<url>.vcs = git` disables Go vanity discovery;
legacy VCS backends are not supported. See [repository URLs](guide.md#cloning-importing-and-archives)
for SSH, GitHub/GitLab, vanity and AWS CodeCommit behavior.

## Choose a manifest

Keep the repositories you want to restore in your dotfiles. TOML, KDL v2 and YAML
represent the same settings and repository entries.

Default directory: `$XDG_CONFIG_HOME/repot/`, or `~/.config/repot/` when
`XDG_CONFIG_HOME` is unset.

| Files present | Selection |
| --- | --- |
| Only `repos.toml` | TOML |
| Only `repos.kdl` | KDL v2 |
| Only `repos.yaml` or only `repos.yml` | YAML |
| More than one default file | An error; choose one explicitly with `--manifest PATH`. |
| None | `repos.toml` is the default when registration creates a manifest. |

```sh
repot --manifest ~/dotfiles/repos.kdl restore --dry-run
repot adopt ~/Downloads/project --manifest ~/dotfiles/repos.toml
```

An explicit logical filename ending in `.kdl` selects KDL v2; `.yaml` and `.yml`
select YAML. Other filenames, including extensionless paths, retain TOML behavior
for compatibility. If the manifest is a symlink, its logical filename controls
the format, not the target's extension.

### TOML

```toml
[settings]
owners = ["your-name", "your-team"]

[[repo]]
url = "https://github.com/your-name/project"

[[repo]]
url = "https://github.com/your-name/dotfiles"
path = "~/.local/share/chezmoi"
restore = false
```

### KDL v2

```kdl
settings {
    owners "your-name" "your-team"
}

repo "https://github.com/your-name/project"
repo "https://github.com/your-name/dotfiles" path="~/.local/share/chezmoi" restore=#false
```

KDL URLs are positional arguments on `repo` nodes. KDL v2 booleans use `#true`
and `#false`. Unknown fields, duplicate settings/properties and incorrect types
are rejected instead of silently ignored.

### YAML

```yaml
settings:
  owners: [your-name, your-team]

repo:
  - url: https://github.com/your-name/project
  - url: https://github.com/your-name/dotfiles
    path: ~/.local/share/chezmoi
    restore: false
```

YAML accepts block and flow mappings in a single document. Unknown fields,
duplicate keys and incorrect types are rejected. Directives, tags, anchors, aliases and merge
keys are not supported; keep entries explicit so registration can preserve them
without resolving indirection.

### Entry fields

| Field | Meaning |
| --- | --- |
| `url` | Required remote URL; use Git's helpers for authentication. |
| `path` | Optional destination or registered location; otherwise use the repository tree. |
| `restore` | Defaults to true. Set false for checkouts managed by another tool. |
| `settings.owners` | Remote owner names used for manual-push recommendations. This never enables automatic pushing. |

Paths expand `~` and environment variables; `$$` represents a literal dollar.
Relative paths resolve against the manifest directory. Registration prefers
home-relative paths when possible. Never place credentials in a URL or manifest.

## Editing and dotfiles

You can edit any of the three formats by hand. `adopt` and `migrate` register repositories
while preserving existing comments. A symlinked manifest is updated at its
dotfiles target without replacing the symlink. A persistent companion `.lock`
file serializes concurrent registrations; updates re-read and atomically replace
the manifest while holding that lock.

`adopt --manifest PATH` can create a new manifest. A repository without a remote
can be registered in place with restoration disabled.

## Restore behavior

`restore` skips entries with restoration disabled and every occupied destination,
including files and dangling symlinks. Missing checkouts are staged privately and
installed with an atomic no-overwrite rename. Independent entries continue after
a clone failure. Submodules are not cloned recursively during restore.

Local absolute/file remotes are supported only when an entry has an explicit
path. Credential-bearing URLs and arbitrary shell-helper protocols are rejected.
See the [user guide](guide.md#find-adopt-and-restore) for commands and safety details.

## Git identities

Use Git's own `includeIf` rules for per-owner email addresses and signing keys.
The `<root>/<host>/<owner>/<repo>` layout makes matching an organisation's tree
straightforward. repot leaves Git authentication, credential helpers and commit
identity configuration with Git.
