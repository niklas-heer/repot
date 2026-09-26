+++
schema_version = 1
id = "01M3FYDYEB1WS7214F9MNKFD4T"
title = "Expose existing safe workflows through typed MCP stdio tools"
date = "2026-09-27"
status = "accepted"
tags = ["architecture", "agents"]
supersedes = []
superseded_by = []
depends_on = []
related_to = ["01M3FP7EBB02S16Q0KFHT100AF", "01M3FQFW55W1EVXPSMJ2ZZHMX7"]
+++
## Decision

Ship a bundled agent guide and an MCP stdio server using the official Rust `rmcp`
SDK. Typed tools invoke the same installed binary through argument arrays, reuse
its safety checks, and return structured command results. Require an explicit
`dry_run` boolean for every mutating tool.

## Context

On 2026-09-27 the user explicitly requested MCP in the first release. Reusing CLI
execution avoids a second implementation of Git and manifest operations. Stdio
fits local coding agents without adding a listening network service. The SDK adds
Tokio only for the server; ordinary commands retain their existing execution model.

## Consequences

MCP adds dependencies and protocol tests. Calls are serialized per server and
command output is bounded. Status defaults to cached refs. Clients must honor user
authorization and treat repository content as data. Cancellation is not rollback;
inspect state before retrying a mutation. Both CLI and MCP documentation ship in
the archive and executable.
