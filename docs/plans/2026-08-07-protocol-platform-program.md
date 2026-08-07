# Protocol and Platform Program

> Status: **Ready for execution / Wave 1 active**
>
> Branch: `program/protocol-platform`
>
> Worktree: `.worktrees/protocol-platform`
>
> Base rule: start from the exact pushed `origin/main` commit supplied by the
> coordinator. Do not continue to another wave until the coordinator merges the
> current wave and supplies a refreshed base.

## 1. Mission

Complete the Protocol and Platform lane of the post-Coding-Tool V2 program:

1. normalize MCP protocol/results and converge transports on one JSON-RPC
   dispatcher;
2. implement Streamable HTTP sessions, retry/cancellation/indeterminate
   semantics, rich content, Tool Artifacts, capability refresh, persistence,
   and product diagnostics;
3. design and implement a Tauri Desktop host that reuses the existing runtime,
   API, ProductStore, canonical events, and Web UI;
4. produce bounded real MCP, browser, Desktop/platform, and external-provider
   evidence when its environment is available.

The branch is long-lived across coordinator-controlled waves, but each wave is
a separate reviewed delivery. Execute only the active wave in this document.

## 2. Required Reading

Read before editing:

1. [`../../AGENTS.md`](../../AGENTS.md)
2. [`../ONBOARDING.md`](../ONBOARDING.md)
3. [`../runtime/README.md`](../runtime/README.md)
4. [`../runtime/subsystems.md`](../runtime/subsystems.md)
5. [`../runtime/implementation-status.md`](../runtime/implementation-status.md)
6. [`../runtime/integration-testing.md`](../runtime/integration-testing.md)
7. [`../design/2026-07-15-mcp-streamable-http-and-tool-artifacts-design.md`](../design/2026-07-15-mcp-streamable-http-and-tool-artifacts-design.md)
8. [`2026-07-25-web-desktop-master-delivery.md`](2026-07-25-web-desktop-master-delivery.md)
9. [`2026-08-07-post-coding-tool-v2-master-program.md`](2026-08-07-post-coding-tool-v2-master-program.md)

Then inspect current MCP transports, product MCP routes, registry assembly,
Execution Environment ports, Tool contracts, tests, and configuration. The
design describes targets; current code and tests remain authoritative.

## 3. Ownership and Boundaries

Primary Wave 1 ownership:

- current MCP proxy/client modules under `runtime/src/tools/`;
- MCP-specific Execution Environment/assembly changes that do not redesign the
  shared environment contract;
- MCP registry assembly under `apps/bootstrap/`;
- `tests/mcp.rs`, `runtime/tests/mcp_contract.rs`, MCP-specific safety/config
  tests, and deterministic transport fixtures;
- current runtime documentation changed by implemented MCP behavior.

Do not edit Runtime Intelligence primary ownership in Wave 1:

- `core/src/` Agent loop or public ToolOutput redesign;
- Runtime engine/planning/StepRunner/Evaluator/Finalizer;
- canonical lifecycle event/state redesign;
- AgentDefinition, `AGENTS.md`, procedure, or OnCall implementation.

Treat public Tool/result types, canonical events, Runtime identity/state schema,
ProductStore/API contracts, root Cargo metadata/lockfiles, and broad Web types
as coordinator hotspots. If a hotspot change is required, stop that part and
report the smallest contract, all producers/consumers, defaults/migration, and
tests to the coordinator. Continue independent MCP work where possible.

## 4. Active Work: Wave 1 MCP Foundation

### 4.1 Characterize the current transports

First map and protect:

- stdio initialize, discovery, calls, timeout, stderr capture, cancellation,
  cleanup, and JSON-RPC errors;
- legacy SSE connection, endpoint negotiation, request/response dispatch, and
  reconnect/failure behavior;
- tool naming, schema validation, registry atomicity, annotations/safety,
  response size bounds, and text ToolOutput projection;
- bootstrap/project trust/config/API/Web probe paths and local/in-memory
  Execution Environment boundaries.

Add deterministic characterization tests for behavior not already protected.

### 4.2 Typed internal protocol and result foundation

Introduce bounded internal MCP types for:

- protocol/session/server/request identity;
- result status, content blocks, structured content, protocol metadata, and
  unknown-block preservation;
- normalized JSON-RPC requests, responses, errors, notifications, and server
  requests;
- conservative safety and typed degradation.

Project these types compatibly into the current public ToolOutput. Do not
redesign `rove-core` ToolOutput, canonical events, persistent artifact schemas,
or UI payloads in Wave 1; propose any needed additive common contract to the
coordinator at handoff.

All remote metadata, schema, content, filenames, MIME values, resource links,
annotations, and errors are untrusted and bounded before projection or logs.

### 4.3 Shared JSON-RPC dispatcher

Implement one dispatcher used by stdio and legacy SSE with:

- opaque unique request IDs and a bounded pending table;
- concurrent out-of-order response correlation;
- notification and server-request routing;
- per-request timeout/cancellation without cross-request corruption;
- EOF/disconnect/parse/protocol error fan-out to pending callers;
- bounded frame/message/diagnostic sizes and clean transport shutdown;
- no blind retry of a request whose remote effect is unknown.

Transport adapters own bytes and connectivity; the dispatcher owns JSON-RPC
semantics. Tool proxy code consumes normalized results rather than parsing
transport-specific payloads.

### 4.4 Identity, safety, and discovery

- Establish stable server, tool, and capability identities without treating a
  connection/session ID as capability identity.
- Map missing/unknown annotations conservatively; text or annotations cannot
  grant permission.
- Support bounded paginated tool discovery, reject duplicate/colliding
  identities, validate the complete catalog, and commit registry changes
  atomically.
- Keep the catalog pinned for a run. Live refresh belongs to a later wave.

### 4.5 Compatibility requirements

- Current stdio and legacy SSE configurations and successful text tool calls
  remain compatible.
- Project Trust, approval policy, ToolRegistry, workspace bounds, secret
  references, and Execution Environment remain the only authorities.
- Restricted workspaces do not read/spawn/probe MCP.
- Secrets and raw upstream bodies do not enter events, errors, reports,
  fixtures, API responses, or snapshots.
- Deterministic tests do not require provider keys, network, or production
  services.
- Streamable HTTP, durable Tool Artifacts, capability refresh, and Desktop are
  not implemented in Wave 1.

## 5. Wave 1 Verification

Run focused checks while iterating, including at least:

```powershell
cargo test -p rove-runtime --test mcp_contract
cargo test -p rove-integration-tests --test mcp
cargo test -p rove-integration-tests --test tool_safety
cargo test -p rove-app-bootstrap
```

Before handoff run:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Run Web tests/typecheck/build only if a currently consumed product contract
changes, and browser E2E only for browser-visible behavior. Real MCP/provider
gates are optional in Wave 1; a skip is not interoperability evidence.

## 6. Wave 1 Exit Gate

Stop and hand back to the coordinator when:

- stdio and legacy SSE use one tested JSON-RPC dispatcher;
- normalized internal protocol/result types preserve current text behavior;
- concurrent response, notification, timeout, cancellation, disconnect,
  malformed/oversized payload, identity collision, pagination, safety, secret,
  and cleanup tests pass;
- registry discovery remains complete, bounded, validated, and atomic;
- current runtime docs match implemented behavior without claiming Streamable
  HTTP or Tool Artifacts;
- all work is committed and pushed to `program/protocol-platform`;
- `git status --short` is clean apart from explicitly reported user-owned
  files.

Do not start Wave 2. Provide commit SHAs, base SHA, changed files, compatibility
story, test exits, unrun optional gates, risks, and any shared-hotspot request.

## 7. Later Waves - Do Not Start Without Refresh

### Wave 2

Implement Streamable HTTP POST JSON/SSE, headers and secret references,
negotiated sessions, GET/DELETE, reconnect, timeout, cancellation, cleanup,
commit-point tracking, bounded retry, and typed indeterminate outcomes with a
deterministic local mock.

### Wave 3

Implement rich content/structured result mapping, the durable bounded Tool
Artifact store, projections, retention and redaction, capability refresh,
atomic catalog replacement, run pinning, checkpoint/resume/report, and API/Web
diagnostics/download safety.

### Wave 4

Seal Desktop D0 first, then implement the Tauri 2 host, shared Web UI delivery,
server-owned runtime/API bridge, bounded filesystem/secret commands, packaging,
platform tests, and external evidence. Do not create Desktop-only Agent loops,
session truth, canonical events, or provider/tool registries.

The coordinator will revise this brief with exact bases and shared contracts at
each barrier.
