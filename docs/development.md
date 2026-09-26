# Development

[← Overview](../README.md) · [Build brief](https://github.com/niklas-heer/repot/blob/main/BUILD_BRIEF.md) · [Design decisions](https://github.com/niklas-heer/repot/tree/main/decisions)

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
`toml`/`toml_edit`, `kdl` and `yaml-edit` for validated, comment-preserving manifests,
`tempfile` for staging,
`nix` for Unix process-group cancellation, and `rustix` for atomic no-overwrite moves.
The existing `clap` dependency handles the command interface.
`ratatui`, `crossterm`, `nucleo-matcher` and `signal-hook` provide the embedded
picker and terminal cleanup. `gix` reads supported Git metadata in-process; Git
remains authoritative for status and mutations. `portable-pty` is a test-only
dependency for actual terminal and shell workflows.
`ureq` with Rustls and `html5gum` implement bounded Go vanity metadata resolution;
`clap_complete` and `clap_complete_nushell` generate shell completions.
`rmcp` supplies the typed stdio MCP protocol and `tokio` its local asynchronous
runtime. MCP operations use the same CLI implementation and safety checks.

`yaml-edit` currently comes from a small vendored runtime copy with a
[checked-negation correction](../vendor/yaml-edit/REPOT_PATCH.md) for a valid
string that made the upstream parser panic. Its license and patch notice ship
with the binary. Repot is distributed through GitHub archives and Git source,
Homebrew and Nix; crates.io publication is disabled while this local dependency
is needed.

Tests invoke the real binary with temporary homes, checkouts and local bare remotes.
Forge responses and failures are controlled by local shims; no test publishes to a
real forge. Four deterministic seeds (`7`, `42`, `2026`, `65537`) run 64 persistent
Git transitions, checking dry-run immutability, commit ancestry, local-file/index
preservation and failure recovery. Failures include the seed and action trace.
Shell tests exercise supported installed shells; Nix checks supply all four.
The picker also has deterministic input-state simulations and PTY tests for
selection, cancellation, resizing, terminal restoration and shell handoffs.
Manifest editing runs seeded sequences against TOML, KDL and YAML, including
concurrent registration, external edits, malformed input and recovery.

## Preparing a release

Keep the Cargo package version and lockfile aligned, write
`docs/releases/vX.Y.Z.md`, and commit the checked changes. Run the **Release**
workflow manually on that commit before creating a tag. Manual runs build and
test all four native targets, verify extracted archives with all three manifest
formats, run the Nix source checks, and install/test the generated Homebrew
formula on a disposable runner. They upload artifacts without publishing.

After the rehearsal passes, push the matching annotated `vX.Y.Z` tag. The
tag workflow repeats the gates and publishes the archives, `SHA256SUMS` and
generated `repot.rb` with the checked-in release notes. Copy that generated
formula into `Formula/repot.rb` after publication and commit it so the tap
installs the released version; the formula also retains `--HEAD` source builds.

## Working with agents

The [agent guide](agents.md), also available through `repot agent-guide`, explains
how an agent can operate the installed CLI: command selection, JSON output, exit
codes and safe automation. It is distinct from this repository’s `AGENTS.md`,
which governs agents contributing to repot’s source.

## Project rules

Use the pinned toolchain and mise tasks, keep dependency features minimal, and
exercise observable behavior through the binary against temporary repositories.
Read [AGENTS.md](https://github.com/niklas-heer/repot/blob/main/AGENTS.md) and the [build brief](https://github.com/niklas-heer/repot/blob/main/BUILD_BRIEF.md) before
contributing. Consequential choices live in [decision records](https://github.com/niklas-heer/repot/tree/main/decisions).
