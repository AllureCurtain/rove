# Project Trust, Execution Environment, and Coding Tools Implementation

> Status: **Active implementation brief - contracts sealed, first parallel wave ready**
>
> Prerequisite implementation: `d2cd822` (`feat(security): gate workspace
> project activation`)
>
> Worktree: `.worktrees/project-trust-execution-tools`
>
> Initial branch: `feature/project-trust-environment-wave1`
>
> Start gate: the branch must be created from the current `origin/main`, must
> contain prerequisite `d2cd822` and this brief, and must have a clean worktree.
>
> This document describes target work. The exact-root activation guard is
> implemented; persistent Project Trust and the Execution Environment are not.

## 1. Objective

Replace the temporary exact-root activation guard with durable, granular
Project Trust, then move filesystem/process/MCP authority behind a Runtime-owned
Execution Environment. After the canonical Tool Schema dependency is merged,
this work also owns Coding Tool V2.

For the first parallel wave, complete only:

1. persistent Project Trust and migration from the temporary guard;
2. local and in-memory Execution Environment foundations;
3. parity migration of current foreground filesystem/search/Shell and stdio MCP
   authority onto the sealed ports.

Stop after the first-wave exit gate. Coding Tool V2 requires the separately
merged canonical Tool Schema and Runtime validation checkpoint.

## 2. Required orientation

Read before editing:

1. [`../../AGENTS.md`](../../AGENTS.md)
2. [`../ONBOARDING.md`](../ONBOARDING.md)
3. [`../runtime/README.md`](../runtime/README.md)
4. [`../runtime/subsystems.md`](../runtime/subsystems.md)
5. [`../runtime/implementation-guide.md`](../runtime/implementation-guide.md)
6. [`../design/2026-07-15-mcp-streamable-http-and-tool-artifacts-design.md`](../design/2026-07-15-mcp-streamable-http-and-tool-artifacts-design.md)

Code and tests remain authoritative when a design example is stale.

## 3. Implemented starting contract

Commit `d2cd822` provides the temporary fail-closed boundary:

- newly selected workspaces are `restricted` by default;
- CLI `--trust-project` grants activation only to the selected canonical root;
- process-level `ROVE_TRUSTED_WORKSPACES` accepts an OS path-list of exact
  canonical roots;
- workspace `.env` and `.rove/config.toml` are deferred until activation;
- workspace files cannot set their own activation grant;
- restricted Engine assembly does not read or spawn workspace MCP servers;
- product MCP listing/editing remains available for inspection, while `probe`
  returns `project_trust_required` before spawn;
- API job-start responses expose `workspace_activation`.

This is a compatibility floor, not the final trust system. Do not remove it
until the persistent replacement passes the same negative tests.

## 4. Sealed Project Trust contract

### 4.1 State and authority

Persist at least these states:

```text
unknown
restricted
trusted
revoked
```

Trust is operator-owned state outside the selected workspace. Repository text,
instructions, tools, MCP annotations, provider output, hooks, and Skills cannot
create or widen a grant. Trust does not replace tool approval.

Grants are independently revocable for at least:

- project configuration;
- workspace instructions;
- MCP/process definitions;
- hooks or executable extensions;
- provider endpoint and credential-name selectors;
- external paths.

### 4.2 Identity and digest

- Bind trust to canonical workspace root, workspace kind, and a stable
  replacement-resistant identity available on the platform.
- Normalize Windows case/verbatim paths, junctions, symlinks, worktrees, drive
  aliases, UNC paths, and nested repositories conservatively.
- Store an executable-configuration digest separately from workspace identity.
- A changed MCP command, hook, external path, provider endpoint, or credential
  selector invalidates only the affected grant.
- A trusted parent never implicitly trusts a nested repository.

### 4.3 Activation and revocation

Activation order is fixed:

```text
canonicalize workspace
  -> load operator-safe bootstrap config
  -> resolve trust record
  -> present bounded requested-capability summary
  -> persist explicit decision
  -> validate project config under operator ceilings
  -> assemble optional executable integrations
```

Revocation blocks new calls immediately. Active processes/MCP sessions are
terminated or quarantined under a documented policy, produce a canonical audit
fact through the existing event workflow, and never auto-restart an unknown
side effect. If that fact requires a new canonical event family, stop and
report the shared-contract requirement.

### 4.4 Surface contract

- CLI and API use the same trust service and stable error codes.
- Web sends workspace IDs, never authority-bearing local paths.
- Trust decision endpoints require existing auth, origin, size, and rate-limit
  protections.
- Restricted state is visible and recoverable, not reported as an internal
  startup error.
- Safe summaries do not expose provider secrets, raw environment values, or
  unnecessary repository content.

## 5. Sealed Execution Environment contract

Runtime owns invocation-scoped capabilities:

```rust
pub trait ExecutionEnvironment: Send + Sync {
    fn identity(&self) -> &ExecutionEnvironmentIdentity;
    fn filesystem(&self) -> &dyn WorkspaceFileSystem;
    fn processes(&self) -> &dyn ProcessHost;
    fn artifacts(&self) -> Option<&dyn ArtifactSink>;
    fn capabilities(&self) -> &ExecutionCapabilities;
}
```

Exact names may follow repository conventions, but these boundaries are fixed:

- tools express intent through Runtime ports;
- only named local adapters call host filesystem/process APIs;
- workspace boundaries are canonical and fail closed;
- cancellation, timeout, output, child cleanup, and unknown-effect semantics
  live at the adapter boundary;
- capability absence is typed and detectable before side effects;
- environment identity is redacted and persisted for resume diagnostics;
- an in-memory adapter supports deterministic conformance tests.

### 5.1 Observation contract

Large reads and process output use bounded observations:

- stable observation identity;
- source, range/cursor, byte count, digest/version, truncation, and artifact
  references;
- no claim that truncated bytes remain in model context;
- stale observation versions cannot authorize mutation;
- product run artifacts and future MCP Tool Artifacts remain distinct types.

## 6. File ownership

Owned in this worktree for the first wave:

- `apps/bootstrap/src/project_trust.rs`
- focused trust/config/assembly/registry changes under `apps/bootstrap/src/`
- new Runtime environment/adapter modules
- `runtime/src/lib.rs`, limited to additive environment module/export wiring
- `runtime/src/tools/`
- focused workspace-boundary changes under `runtime/src/workspace/`
- trust-specific API routes/contracts and ProductStore tables/migrations
- trust and environment Settings modules under `apps/web/settings/`
- strict Web parsers/types for trust-specific API fields
- `tests/tool_safety.rs`
- `tests/mcp.rs`
- trust-focused cases in `tests/api.rs`
- new narrowly named trust/environment integration tests
- `docs/runtime/subsystems.md`
- `docs/runtime/implementation-guide.md`
- `docs/runtime/integration-testing.md`

Do not modify during the first wave:

- `models/`
- `core/`
- provider-neutral message/session types
- Runtime Agent/model/planning loops
- canonical event families or broad `TaskState` schema
- unrelated ProductStore entities/migrations
- root `Cargo.toml` or `Cargo.lock`
- `PRODUCT_ACCEPTANCE_REPORT.json`
- current runtime documents other than the three explicitly owned above

Trust-specific public API/OpenAPI/Web fields and trust-store migrations are
assigned here. If an audit event requires a new canonical event family, stop
and report the shared hotspot rather than editing it.

## 7. First parallel wave

### Checkpoint 1 - Persistent Project Trust

- Add the operator-owned trust repository and schema migration.
- Implement canonical identity, exact-root lookup, requested-capability digest,
  explicit grant, denial, and revocation.
- Migrate the temporary CLI/environment grants without silently converting
  history into durable trust.
- Gate project config, local `.env`, MCP probe/registration, and other existing
  executable activation through the service.
- Add CLI/API/Web restricted/trusted/revoked states and explicit decisions.
- Preserve safe workspace inspection before trust.

Exit:

- opening/listing an unknown workspace starts no repository-owned process;
- config cannot replace provider endpoint/credential authority before trust;
- changed executable config invalidates the affected grant;
- revocation blocks new activation and handles active processes conservatively;
- auth/origin negative tests protect trust decisions;
- alias, nested-repository, symlink, junction, and replacement fixtures fail
  closed on supported platforms;
- all `d2cd822` negative tests still pass.

### Checkpoint 2 - Execution Environment parity

- Add `ExecutionEnvironment`, filesystem, process, capability, and identity
  ports with local and in-memory adapters.
- Inject the environment through Runtime-owned invocation services.
- Move current file/search operations to `WorkspaceFileSystem`.
- Move current foreground Shell behavior to `ProcessHost`.
- Route stdio MCP spawn and child cleanup through the process capability.
- Add the bounded observation store and conformance suite.
- Preserve existing tool names, request schemas, approval behavior, outputs,
  events, mutations, timeouts, and cancellation behavior.

Exit:

- built-in tools contain no direct host filesystem/process calls outside named
  adapters;
- local and in-memory adapters pass the same conformance suite;
- current file/search/Shell/MCP integration tests remain behaviorally equal;
- path, symlink/junction, timeout, cancellation, cleanup, and output bounds pass;
- resume diagnostics contain only redacted environment identity.

## 8. Required verification

Run focused checks first, then all affected Rust/Web gates:

```powershell
cargo fmt --all --check
cargo test -p rove-app-bootstrap
cargo test -p rove-runtime --test tool_contract
cargo test -p rove-runtime --test mcp_contract
cargo test -p rove-integration-tests --test tool_safety
cargo test -p rove-integration-tests --test mcp
cargo test -p rove-integration-tests --test api
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

Set-Location apps/web
pnpm test
pnpm typecheck
pnpm build
```

Run browser E2E when trust decisions or restricted-state UI flows change. Real
MCP/provider gates remain opt-in; a skipped gate proves only its skip path.

## 9. Handoff and stop condition

Produce separate commits for persistent trust and Execution Environment parity.
At handoff report:

- commit SHAs and exact base SHA;
- schema migration and rollback behavior;
- files changed;
- negative/security tests and real exit codes;
- Web/API compatibility changes;
- current runtime documentation updated for behavior changed in this wave;
- optional gates not run;
- unresolved shared-hotspot requests;
- clean `git status --short`.

Do not begin Coding Tool V2 until the canonical Tool Schema/Runtime-validation
dependency is merged into `main` and a refreshed baseline is supplied.

## 10. Later owned work after refresh

Coding Tool V2 remains assigned here after its dependency lands:

- ranged, bounded Read with observations and continuation;
- exact Edit with uniqueness and stale-version checks;
- create-first Write plus explicit compatible overwrite;
- observed delete/move/rename and bounded directory operations;
- localized Diff and bounded checkpoint/rewind;
- deterministic search/list/glob continuation;
- foreground/background Shell identity and progressive output;
- explicit PTY support or typed unsupported capability;
- large-output/artifact projection and context reclamation;
- deterministic coding benchmark coverage.
