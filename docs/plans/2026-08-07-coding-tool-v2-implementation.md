# Coding Tool V2 Implementation

> Status: **Implemented / Verified**
>
> Branch: `feature/coding-tool-v2`
>
> Base: `acc82e2` (`feat(runtime): validate authoritative tool schemas`)
>
> Prerequisites: persistent Project Trust, Runtime-owned Execution Environment,
> authoritative Tool Schema validation, deterministic registry catalogs, and
> immutable Capability Snapshots are implemented on the base.

## 1. Objective

Implement a bounded coding-tool contract on the existing Runtime-owned
filesystem and process ports. The work must make stale or unobserved mutations
fail closed, keep large results out of active model history, and preserve the
shared Runtime, approval, workspace, registry, and capability-snapshot paths.

This plan implements Coding Tool V2 only. It does not implement the shared
Agent kernel, model evaluator/Finalizer, MCP Streamable HTTP or rich MCP result
envelopes, AgentDefinition/`AGENTS.md` discovery, Desktop, or live capability
refresh.

## 2. Starting Contract

The base already provides:

- `WorkspaceFileSystem`, `ProcessHost`, local/in-memory execution adapters, and
  bounded foreground process capture;
- a bounded `ObservationStore` metadata foundation that is not yet wired into
  tools;
- `read_file`, overwrite-style `write_file`, deterministic-but-non-continuable
  `search_code`, and foreground-only `run_shell`;
- authoritative pinned `ToolDescriptor` values and immutable Runtime
  `CapabilitySnapshot` identity;
- Project Trust and ordinary destructive-tool approval as independent gates.

The base does not provide ranged reads, version-bound mutations, filesystem
checkpoint/rewind, background process identity, progressive process output, or
large-result projection.

## 3. Implemented Surface

### 3.1 Observations and bounded projection

- Execution Environment owns one bounded observation/artifact store per Engine.
- File versions are SHA-256 digests computed under an explicit maximum source
  size. Observations bind source, range/cursor, bytes, version, truncation, and
  an optional transient artifact reference.
- `read_file` accepts bounded offset/limit/continuation. A V2 request returns a
  structured projection with an observation ID, version, exact range, and next
  continuation. The legacy `{ "path": ... }` call keeps plain-text output for
  a complete small UTF-8 file; a large legacy read is projected instead of
  entering history unbounded.
- Observation/artifact retention is byte-, item-, and per-entry-bounded. It is
  invocation-environment state, not the proposed MCP Tool Artifact envelope or
  a new durable event family.

### 3.2 File mutation contract

- `edit_file` requires a file observation ID and version, verifies the current
  source version, requires the exact old text to occur once in the observed
  source, then records a localized mutation diff.
- `write_file` is create-first. Writing an existing path fails unless
  `mode = "overwrite"` is explicit. The existing tool name and create request
  shape remain compatible; callers that relied on implicit overwrite must add
  the explicit mode. Optional observation/version fields protect compatible
  overwrite from stale content.
- `delete_path` and `move_path` require an observation whose source and version
  still match. Directory mutation requires one complete, non-truncated
  recursive directory observation and remains entry-bounded. Move does not
  overwrite an unobserved destination.
- File and directory paths continue through canonical workspace boundary
  checks. All mutation tools remain destructive, serialized, and subject to
  approval. Negative traversal, stale-version, ambiguity, and unobserved-target
  tests are required.

### 3.3 Deterministic discovery

- `list_directory` and `glob_paths` return lexical, workspace-relative results
  with bounded page sizes and continuation.
- `search_code` preserves its existing fields and adds deterministic lexical
  continuation, observation/version metadata, and request-bound cursor
  validation.
- Continuations fail closed when their source/request binding is invalid or
  stale; callers restart discovery after workspace changes.

### 3.4 Diff, checkpoint, and rewind

- `workspace_checkpoint` snapshots an explicit path set or one bounded
  workspace catalog into Execution Environment memory.
- `workspace_diff` returns localized, size-capped diffs against that checkpoint
  and supports an explicit bounded path filter.
- `workspace_rewind` restores only explicitly selected checkpoint paths, caps
  file count and bytes, records mutations, and requires ordinary destructive
  approval.
- Checkpoints intentionally do not survive Engine recreation or process
  restart. A missing checkpoint is a typed failure; resume never silently
  substitutes another snapshot.

### 3.5 Shell lifecycle

- `run_shell` preserves foreground behavior and adds explicit background start.
- Background execution returns an opaque Runtime process ID. `shell_output`
  reads bounded progressive stdout/stderr pages using independent cursors, and
  `shell_terminate` performs bounded kill-and-wait cleanup.
- Poll results expose whether either retained stream has more data and whether
  both streams are closed. Terminal identities are reclaimed after both streams
  are drained; explicit termination releases immediately.
- Process output is drained under a fixed retention ceiling even when projected
  text is smaller. Unknown, completed, cancelled, timed-out, and truncated
  states remain explicit.
- PTY is represented by a registered typed unsupported capability on this
  platform-independent wave. No pseudo-terminal interoperability claim is
  made.

### 3.6 Capability and compatibility contract

- New built-ins have stable capability IDs and enter the existing registry and
  Capability Snapshot. Capability IDs never grant permission.
- `ExecutionCapabilities` gains additive serde-defaulted fields for background
  process, PTY, checkpoint, and artifact-projection availability. The redacted
  Product runtime endpoint and strict Web parser are updated together.
- Existing runtime identity artifacts without the new fields remain readable.
- Tool result lifecycle and `ToolResult` serialization remain unchanged; V2
  metadata is carried in bounded tool output content and existing mutation
  records.

## 4. Implementation Checkpoints

### Checkpoint A - Environment primitives

- Wire bounded observations plus transient artifact payloads into each
  Execution Environment.
- Add versioned/ranged filesystem, observed mutation, deterministic catalog,
  checkpoint/diff/rewind, and background process lifecycle ports.
- Extend local/in-memory conformance tests before registering tools.

Exit: both adapters pass the same bounds, stale-version, continuation,
checkpoint, and missing-capability tests; local timeout/cancel/drop cleanup
remains green.

### Checkpoint B - File/discovery tools

- Implement ranged `read_file`, exact `edit_file`, create-first `write_file`,
  observed delete/move, list/glob, continued search, diff/checkpoint/rewind.
- Register stable descriptors and add approval/path/stale negative tests.

Exit: all outputs are bounded, ordering and continuations are deterministic,
and no mutation succeeds from an unobserved or stale target.

### Checkpoint C - Shell/projection tools

- Add background start, progressive polling, termination, output projection,
  and typed unsupported PTY.
- Prove process identity isolation, cursor bounds, timeout/cancellation, and
  cleanup.

Exit: foreground compatibility remains green; background output can be drained
incrementally without copying retained output into every model turn.

### Checkpoint D - Benchmark and product contracts

- Add a deterministic no-network coding benchmark that exercises read/edit,
  create/overwrite, discovery continuation, checkpoint/diff/rewind, and
  background Shell.
- Update Product runtime capability API/OpenAPI types, strict Web parsing, and
  current runtime docs.

Exit: the benchmark runs through the real Engine/tool/state/artifact path and
checks exact workspace results plus canonical trace/report artifacts.

## 5. Verification

Run focused checks first:

```powershell
cargo test -p rove-runtime --test environment_contract
cargo test -p rove-runtime --test tool_contract
cargo test -p rove-integration-tests --test tool_safety
cargo test -p rove-integration-tests --test bench
cargo test -p rove-integration-tests --test api
```

Then expand:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

Set-Location apps/web
pnpm test
pnpm typecheck
pnpm build
```

Browser E2E is required only if a browser-visible flow changes beyond additive
runtime capability fields. External provider, real MCP, and native PTY gates
remain opt-in and cannot be claimed from a skip.

## 6. Acceptance Conditions

- Read, search, list, glob, and process output are byte/item/time bounded and
  expose valid continuation without duplicate pages.
- Exact edit rejects zero/multiple matches, mismatched observation sources, and
  stale versions before mutation.
- Write creates by default and overwrites only through the explicit compatible
  mode; traversal and symlink/junction escape tests remain green.
- Delete/move/rename cannot operate on an unobserved or stale target; recursive
  directory work requires a complete bounded observation, and workspace-root
  mutation is always rejected.
- Diff is localized and capped; rewind changes only requested checkpoint paths
  and records existing mutation metadata.
- Background processes have opaque identity, progressive output, terminal
  status, bounded termination, and environment-drop cleanup.
- PTY absence is visible as typed unsupported capability, not a generic host
  error or false interoperability claim.
- Large results are projected through bounded observations/artifact references
  and active model history receives only the selected page.
- The deterministic coding benchmark passes without provider keys or network.
- Runtime docs describe the implemented behavior and retain explicit deferred
  boundaries.
- Focused tests, formatting, Clippy, workspace tests, and affected API/Web gates
  pass before commit and push.

## 7. Verification Result

Implemented checkpoints A-D are covered by local/in-memory environment tests,
focused tool contracts, integration safety/API/E2E tests, and the deterministic
`coding-tool-v2` benchmark. The suite executes 13 real Engine tool calls with
zero tool failures and checks exact final files plus canonical
`trace.jsonl`/`task_state.json`/`report.json` artifacts.

Passing gates:

```powershell
cargo test -p rove-runtime --test environment_contract --test tool_contract
cargo test -p rove-integration-tests --test tool_safety
cargo test -p rove-integration-tests --test bench
cargo test -p rove-integration-tests --test e2e
cargo test -p rove-integration-tests --test api
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

Set-Location apps/web
pnpm test
pnpm typecheck
pnpm build
```

The browser E2E suite was not required because the Web change is an additive
strict-parser/runtime-capability contract with no new browser interaction.
External-provider, real MCP, and native PTY gates remain opt-in and were not
run; no interoperability claim is made from those skips.
