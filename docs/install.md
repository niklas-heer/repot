# Installation

[← Overview](../README.md) · [User guide](guide.md)

## Requirements

repot supports Linux and macOS on x86-64 and ARM64. The fuzzy picker is included
in the binary. Git provides authentication, networking and authoritative checkout
operations. GitHub CLI (`gh`) or GitLab CLI (`glab`) is needed for publishing;
`gh` can also supply optional exact merged-PR evidence for safe branch return.

AWS CodeCommit URLs additionally need Git's CodeCommit helper. AWS CLI is needed
only if region selection falls back to `aws configure get region`.

## Homebrew

```sh
brew tap niklas-heer/repot https://github.com/niklas-heer/repot
brew install niklas-heer/repot/repot
```

To build the current development source instead:

```sh
brew install --HEAD niklas-heer/repot/repot
```

The stable formula uses release archives and their actual SHA-256 hashes.
Homebrew also supplies Git.

## Nix

The locked flake builds from source with the pinned Rust toolchain and supplies
Git at runtime.

```sh
nix profile install github:niklas-heer/repot/v0.1.0
nix run github:niklas-heer/repot/v0.1.0 -- --help
```

From a checkout, use `nix build .` or `nix run . -- --help`.

## Prebuilt archives

Download your platform's archive and `SHA256SUMS` from the
[v0.1.0 release](https://github.com/niklas-heer/repot/releases/tag/v0.1.0).

| Platform | Archive target |
| --- | --- |
| macOS Apple Silicon | `aarch64-apple-darwin` |
| macOS Intel | `x86_64-apple-darwin` |
| Linux ARM64 | `aarch64-unknown-linux-gnu` |
| Linux Intel/AMD | `x86_64-unknown-linux-gnu` |

Archives are named `repot-0.1.0-TARGET.tar.gz`. Linux archives target glibc.
Verify before extracting. For example, on an Apple Silicon Mac:

```sh
grep '  repot-0.1.0-aarch64-apple-darwin.tar.gz$' SHA256SUMS | shasum -a 256 -c -
tar -xzf repot-0.1.0-aarch64-apple-darwin.tar.gz
mkdir -p ~/.local/bin
install -m 755 repot ~/.local/bin/repot
```

On Linux, select your archive’s line from `SHA256SUMS` and pipe it to `sha256sum -c -`. Ensure
`~/.local/bin` is on your `PATH`. Archives include the README, license, icon and
user documentation. The release also contains aggregate `SHA256SUMS` and a
`repot.rb` Homebrew formula.

## Build from source

Install [mise](https://mise.jdx.dev/), then:

```sh
git clone https://github.com/niklas-heer/repot
cd repot
mise install rust
mise exec -- cargo install --locked --path .
```

This uses the project's pinned Rust version. For contribution and CI commands,
see [Development](development.md).

## Shell integration

A child process cannot change its parent shell's directory. These wrappers follow
the directory handoff only after a successful repot command. Without a wrapper,
`jump` prints the selected path.

### Bash and Zsh

Add the appropriate line to `~/.bashrc` or `~/.zshrc`:

```sh
eval "$(repot shell-init bash)"
# Zsh: eval "$(repot shell-init zsh)"
```

### Fish

Add this to `~/.config/fish/config.fish`:

```fish
repot shell-init fish | source
```

### Nushell

Generate a file once from Nushell:

```nu
^repot shell-init nu | save --force ($nu.default-config-dir | path join "repot.nu")
```

Then add this line to `config.nu` and start a new shell:

```nu
source repot.nu
```

Regenerate the file after upgrading repot to pick up wrapper changes.

### Completions

`repot completions bash`, `zsh`, `fish` or `nu` prints the corresponding completion
script. Save and load it using your shell's normal completion convention. For
example, in Bash:

```sh
eval "$(repot completions bash)"
```
