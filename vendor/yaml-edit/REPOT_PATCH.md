# yaml-edit 0.3.2 local correction

This directory contains the runtime source and README of yaml-edit 0.3.2,
authored by Jelmer Vernooĳ and contributors and licensed under Apache-2.0.
Upstream: https://github.com/jelmer/yaml-edit
Published source: https://crates.io/crates/yaml-edit/0.3.2
Registry archive SHA-256: 79e531de3d80075e0a2dc8246ca46c9167211297f8b9d513130880c2ef98d0b8
The published VCS revision is retained in UPSTREAM_VCS.json.

Repot modification, 2026-09-27: `ScalarValue::parse_integer` uses
`checked_neg()` instead of unchecked negation when applying a leading minus.
A valid plain YAML string such as `--9223372036854775808` previously panicked
while the lexer attempted integer classification. Failed negation now returns
`None`, allowing that value to remain a string. No panic hook or source
rewriting is needed. `tests/manifest_yaml.rs` exercises the actual repot binary
with this scalar, quoted and unquoted, through reading and registration.

The local manifest omits upstream development dependencies and benchmark
entries; only runtime sources and README are bundled. `publish = false`
prevents accidental publication of this local copy. The Apache-2.0 license is
included as LICENSE; upstream's published crate did not include a NOTICE file.

Remove this vendor copy and return to a registry dependency once an upstream
release fixes the checked-negation failure and passes the repot YAML grammar,
comment-preservation and deterministic editing tests. Do not replace it with
the unpatched registry version merely to make Cargo packaging succeed. Repot
ships GitHub binary archives and Git source; it is not published to crates.io.
