# rove Implementation Guide

This guide is for maintainers who need to understand, debug, or extend the current implementation. It describes what exists in the codebase today. Product intent and historical design rationale live in the top-level docs; the current runtime source of truth remains this `docs/runtime/` directory.

## 1. Runtime Shape

`rove` is a local-first agent runtime with three user-facing shells:

```text
CLI / API / Web
    -> Engine
        -> ContextManager
        -> ModelClient / RoutingModelClient
        -> Executor / ToolRegistry
        -> Memory loaders and hooks
        -> StateStore

StateStore
    -> .rove/runs/<run_id>/*
    -> .rove/state.sqlite
```

The interface layers construct the runtime and consume `StreamEvent` values. Core code does not depend on CLI, API, or Web modules.

Important entry points:

| Area | Files |
|---|---|
| Library module tree | `src/lib.rs` |
| CLI binary | `src/main.rs`, `src/interfaces/cli/*` |
| API binary | `src/bin/rove-api.rs`, `src/interfaces/api/mod.rs` |
| Web workbench | `web-ui/` |
| Engine and runtime types | `src/core/*` |
| State artifacts and SQLite index | `src/state/*` |
| Model providers | `src/models/*` |
| Tools and MCP/RAG adapters | `src/tools/*` |
| Memory hooks and stores | `src/memory/*`, `src/hooks/*` |

## 2. Workspace Detection

`Workspace::detect` is the first runtime boundary. It canonicalizes the starting directory, walks upward for `.git`, and returns either:

- `WorkspaceKind::Repo` with the nearest git root as `workspace.root`;
- `WorkspaceKind::Folder` with the starting directory as `workspace.root`.

The default state directory is `workspace.root/.rove`. Config can override `state.state_dir` and `state.sqlite_path`. Core state and memory tools follow `workspace.state_dir`; RAG still has `.rove` path assumptions, so check the maintenance notes before relying on custom RAG paths.

Relevant code:

- `src/core/workspace.rs`
- `src/config.rs`

## 3. Configuration

`AppConfig::load` merges configuration in this order:

```text
defaults < .rove/config.toml < environment < CLI/API overrides
```

The config is grouped by runtime, provider, tool, memory, state, API, web, and routing. `dump-config` prints the effective config, source summary, resolved paths, and redacted secret presence flags.

Common paths and defaults:

| Config | Default |
|---|---|
| `runtime.system_prompt_path` | `prompts/system.md` |
| `state.state_dir` | `.rove` |
| `state.sqlite_path` | `.rove/state.sqlite` |
| `tool.mcp_config_path` | `.rove/mcp_servers.json` |
| `memory.session_dir` | `.rove/memory/sessions` |
| `memory.durable_dir` | `.rove/memory` |
| `api.bind_addr` | `127.0.0.1:8787` |

Remote API binding is rejected unless token auth is configured or `api.unsafe_remote_without_auth = true` is set.

Useful commands:

```powershell
cargo run -- dump-config
```

Relevant code:

- `src/config.rs`
- `src/interfaces/cli/config.rs`

## 4. CLI Startup Path

The CLI binary handles maintenance subcommands first, then one-shot agent runs.

High-level flow in `src/main.rs`:

1. Parse `Args` from `src/interfaces/cli/args.rs`.
2. Run early subcommands:
   - `dump-config`
   - `index`
   - `sessions`
   - `state repair`
   - `state cleanup`
3. Detect workspace.
4. Load `AppConfig`.
5. Construct the model client.
6. Register the shared default tool registry and MCP tools.
7. Build `ContextManager`.
8. Build `Engine`.
9. Create `StateStore` and `RunHandle`.
10. Resolve optional CLI resume state.
11. Run `run_oneshot_with_cancel`.

Current one-shot smoke command:

```powershell
cargo run -- --model fake "echo hello from rove"
```

`Cargo.toml` sets `default-run = "rove"`, so plain `cargo run -- ...` uses the CLI binary.

Relevant code:

- `src/main.rs`
- `src/interfaces/cli/args.rs`
- `src/interfaces/cli/oneshot.rs`
- `src/interfaces/cli/sessions.rs`
- `src/interfaces/cli/state.rs`
- `src/interfaces/cli/index.rs`
- `src/state/resume.rs`

## 5. API Startup Path

The API binary is thin. `src/bin/rove-api.rs` parses an optional bind address and working directory, then calls `serve`.

`serve_with_shutdown`:

1. Detects the workspace.
2. Loads config with API bind override.
3. Applies configured `state_dir`.
4. Creates `ApiState`.
5. Initializes the SQLite index.
6. Marks stale `init` or `running` jobs as `interrupted`.
7. Binds the TCP listener and serves the router.

Routes:

| Route | Purpose |
|---|---|
| `POST /jobs` | Create and start a job |
| `GET /jobs/{job_id}/state` | Read live or persisted job state |
| `GET /jobs/{job_id}/events` | Stream SSE events, with replay support |
| `POST /jobs/{job_id}/cancel` | Cancel a live job |
| `POST /jobs/{job_id}/approvals/{call_id}` | Resolve a pending destructive tool approval |
| `POST /jobs/{job_id}/inputs/{input_id}` | Resolve a pending `request_input` prompt |

API jobs have two state layers:

- live handles in memory: task handle, cancellation token, broadcast sender, approval/input channels;
- durable state in SQLite and `.rove/runs/<run_id>/`.

`POST /jobs` accepts `message`, optional `model`, `max_steps`, `approval`, and optional `resume`.
`resume` follows the CLI semantics: omit it for a fresh session/job, use `"latest"` for the newest task snapshot, or pass a run id to load that exact snapshot. A resumed API job keeps the loaded `session_id` and `job_id`, creates a new `run_id`, and passes the loaded `TaskState` into `RunRequest` and artifact recording.

After restart, historical job state and SSE events can be read from SQLite. Pending approvals and pending inputs are intentionally not reconstructed.

Relevant code:

- `src/bin/rove-api.rs`
- `src/interfaces/api/mod.rs`
- `src/interfaces/api/security.rs`

## 6. Web Workbench Path

`web-ui/` is a standalone Next.js app. It talks to the Rust API through a local rewrite:

```text
/api/* -> ROVE_API_BASE or http://127.0.0.1:8787
```

The main component:

1. Creates a job with `POST /jobs`.
2. Opens an `EventSource` for `/jobs/{job_id}/events`.
3. Applies streamed events through `workbenchReducer`.
4. Calls approval and input endpoints when user action is required.
5. Fetches job state on stream errors to resync.

Relevant code:

- `web-ui/components/rove-workbench.tsx`
- `web-ui/lib/rove-client.ts`
- `web-ui/lib/rove-state.ts`
- `web-ui/lib/rove-types.ts`
- `web-ui/next.config.mjs`

Current Web checks:

```powershell
cd web-ui
npm test
npm run typecheck
npm run build
```

Browser-level end-to-end tests are not part of the default CI surface.

## 7. Core Runtime Types

The core type model is centered on explicit IDs and serializable runtime state:

| Type | Purpose |
|---|---|
| `SessionId` | User-level continuity across jobs |
| `JobId` | One submitted task |
| `RunId` | One engine execution |
| `CallId` | One tool call |
| `RunRequest` | Identity + user message + optional resume state |
| `TaskState` | Serializable resume snapshot |
| `PromptCheckpoint` | Compact reconstruction point for resume |
| `TaskPlan` | Planner output and current step pointer |
| `Message` | Provider-facing conversation message |
| `ToolSchema` | Tool contract exposed to the model |
| `RunStatus` | API/job status |
| `TerminationReason` | Engine completion reason |

Relevant code:

- `src/core/types.rs`

## 8. Stream Events

`Engine::run_with_cancel` returns a stream of `StreamEvent` values. Consumers should treat the event stream as the public runtime protocol.

Current event variants:

- `run_started`
- `llm_chunk`
- `llm_message`
- `tool_call_started`
- `tool_call_approval_needed`
- `tool_call_completed`
- `tool_call_failed`
- `input_needed`
- `plan_created`
- `plan_step_started`
- `plan_step_completed`
- `plan_step_failed`
- `run_completed`

The API serializes these events as SSE using `StreamEvent::event_name()`. The trace writer serializes the same events to `trace.jsonl` and indexes them in SQLite with sequence numbers.

Adding a new event requires checking:

- CLI rendering in `src/interfaces/cli/oneshot.rs`
- API SSE/event persistence in `src/interfaces/api/mod.rs` and `src/state/index.rs`
- Web types and reducer in `web-ui/lib/rove-types.ts` and `web-ui/lib/rove-state.ts`
- artifact recording in `src/state/artifacts.rs` if it affects resume/report state

Relevant code:

- `src/core/events.rs`
- `src/state/trace.rs`

## 9. Engine Execution Flow

`Engine` owns the model client, tool registry, context manager, workspace, approval policy, hooks, memory recall limit, and optional interface providers for approval/input.

The high-level run flow:

1. Emit `RunStarted`.
2. Build history from resume checkpoint or full resume state.
3. Load durable/session memory into working prompt memory.
4. If planning is enabled:
   - draft or resume a `TaskPlan`;
   - emit `PlanCreated`;
   - loop over plan steps;
   - build step-specific context;
   - call the model;
   - execute tool calls or mark final output;
   - replan on malformed output or tool failure.
5. If planning is disabled:
   - run the simpler ReAct loop over the original user message.
6. Emit `RunCompleted`.
7. Run post-run hooks before the stream closes.

Termination can happen because of:

- final answer;
- step limit;
- token hard limit;
- model error;
- planner error;
- cancellation.

The planned and unplanned paths currently duplicate tool-call handling logic. If you are changing approval, batch execution, native tool-use, or history mutation, check both branches.

Relevant code:

- `src/core/engine.rs`
- `src/core/planner.rs`
- `src/core/context.rs`
- `src/core/parser.rs`

## 10. Context And Compaction

`ContextManager` builds provider messages from:

```text
system -> durable memory -> session memory -> compact summary -> recent history tail -> current user message
```

There are two modes:

- message-count history limit;
- token-budget history limit.

Token estimates are approximate: four characters per token plus message/tool-call overhead. The context builder reports whether it crossed soft/hard budgets and whether automatic compaction is needed.

Compaction is deterministic today. `RunArtifactRecorder` writes `PromptCheckpoint` with:

- optional summary;
- preserved tail messages;
- current plan;
- memory pointers;
- last step;
- token estimate;
- compacted message count;
- compaction metadata.

Resume prefers checkpoint tail/summary over replaying the full saved history.

Relevant code:

- `src/core/context.rs`
- `src/state/artifacts.rs`

## 11. Model Layer

All providers implement:

```rust
trait ModelClient {
    fn stream(&self, messages: &[Message], tools: &[ToolSchema])
        -> BoxStream<'_, Result<ModelEvent, ModelError>>;

    fn model_id(&self) -> &str;

    fn client_id(&self) -> ModelClientId;
}
```

Provider adapters normalize provider-specific streaming responses into `ModelEvent`:

- `TextDelta`
- `ThinkingDelta`
- `ToolUseStart`
- `ToolUseDelta`
- `ToolUseDone`
- `Usage`
- `Done`

Native providers:

| Provider | File |
|---|---|
| OpenAI-compatible | `src/models/openai.rs` |
| Anthropic | `src/models/anthropic.rs` |
| Ollama | `src/models/ollama.rs` |
| Fake | `src/models/fake.rs` |

`RoutingModelClient` wraps a primary model plus fallback models/providers. It can fall back only before committed visible output or committed tool-use. Provider target identity is provider plus endpoint plus model, exposed as `ModelClientId`, so two providers using the same model name do not share a health bucket.

`src/models/health.rs` owns `ModelHealthStore`, `HealthConfig`, and circuit state. CLI-created routed clients keep private health state configured from `routing.failure_threshold` and `routing.open_cooldown_ms`. API state creates one process-shared `ModelHealthStore` and injects it into routed model clients so API jobs share circuit breaker decisions across runs in the same process.

First-packet routing decisions are emitted through `tracing`: candidate start, skipped open circuit, committed first event, no content, timeout, and error-before-commit. These are observability records only; they do not add user-facing `StreamEvent` variants.

Relevant code:

- `src/models/traits.rs`
- `src/models/factory.rs`
- `src/models/routing.rs`
- `src/errors.rs`

## 12. Tool System

Tools implement `Tool` and are registered in `ToolRegistry`. The registry exposes schemas to the model and dispatches execution by name.

Current built-in tools:

| Tool | Purpose |
|---|---|
| `echo` | Deterministic smoke/demo tool |
| `fs_read` | Read UTF-8 workspace file |
| `fs_write` | Write UTF-8 workspace file |
| `shell` | Run shell command in workspace |
| `save_memory` | Save durable memory topic |
| `update_memory_index` | Rebuild durable memory index |
| `read_memory_topic` | Read durable memory topic |
| `request_input` | Ask user/interface for mid-run input |
| `retrieve_code` | RAG code retrieval or stub |
| `retrieve_docs` | RAG docs retrieval or stub |
| `mcp__<server>__<tool>` | MCP-proxied remote tools |

CLI and API both construct built-ins through `default_tool_registry(&Workspace)`. Root-bound tools receive the workspace root at construction. Memory tools are context-bound and derive their paths from `ToolContext.workspace.state_dir`.

Tool schemas include:

- `destructive`: requires approval unless policy allows it;
- `parallel_safe`: allows concurrent batch execution if every call is non-destructive and safe.

The executor pipeline is currently:

```text
schema lookup -> argument validation -> pre-tool hooks -> permission -> execute -> result wrapping -> post-tool hooks
```

The historical docs mention a diff/write-set phase. That is not implemented as a separate pipeline stage yet.

Relevant code:

- `src/tools/traits.rs`
- `src/tools/registry.rs`
- `src/core/executor.rs`
- `src/core/boundary.rs`
- `src/hooks/mod.rs`

## 13. Approval And Input

Approval is controlled by `ApprovalPolicy`:

- `ask`: destructive tools emit approval-needed events and wait for a decision;
- `auto`: destructive tools run without prompting;
- `never`: destructive tools are blocked.

Interfaces provide decisions through `ToolApprovalProvider`.

For API jobs, pending approvals live in the live `JobRecord` and are exposed through:

```text
POST /jobs/{job_id}/approvals/{call_id}
```

`request_input` uses `UserInputProvider`. For API jobs, pending inputs are exposed through:

```text
POST /jobs/{job_id}/inputs/{input_id}
```

Pending approvals/inputs are live-only. They are shown in live job state, but not reconstructed after process restart.

Relevant code:

- `src/core/types.rs`
- `src/interfaces/cli/approval.rs`
- `src/interfaces/cli/input.rs`
- `src/interfaces/api/mod.rs`
- `src/tools/request_input.rs`

## 14. State Artifacts

Each run writes readable files under:

```text
.rove/runs/<run_id>/
  trace.jsonl
  task_state.json
  report.json
```

`trace.jsonl` is append-only event history. Every line is one serialized `StreamEvent`.

`task_state.json` is the resume snapshot. It includes:

- identity;
- goal;
- step count;
- conversation history;
- summary;
- prompt checkpoint;
- plan state.

`report.json` is the final aggregate report. It includes:

- identity;
- workspace metadata;
- model id;
- final status;
- termination reason;
- step count;
- total usage;
- tool counts;
- final output.

Relevant code:

- `src/state/store.rs`
- `src/state/trace.rs`
- `src/state/artifacts.rs`
- `src/state/report.rs`

## 15. SQLite State Index

SQLite is the query/replay index, not the only source of truth. Files remain the readable artifacts.

The index stores:

- sessions;
- jobs;
- runs;
- events and event offsets;
- reports;
- task state metadata;
- pending approval/input schema slots;
- migrations.

Startup and maintenance behavior:

- `StateIndex::initialize` creates/migrates the database.
- API startup marks stale running jobs as `interrupted`.
- task states can be lazily imported from artifacts.
- `rove state repair` imports task state artifacts explicitly.
- `rove state cleanup` removes expired rows and safe run artifacts.

Useful commands:

```powershell
cargo run --bin rove -- sessions
cargo run --bin rove -- state repair
cargo run --bin rove -- state cleanup
```

Relevant code:

- `src/state/index.rs`
- `src/state/store.rs`
- `src/interfaces/cli/state.rs`

## 16. Memory

Memory has three layers:

| Layer | Storage | How it is used |
|---|---|---|
| Working memory | in-memory prompt messages | Included in the current context |
| Session memory | `.rove/memory/sessions/<session_id>.md` | Loaded on resume / same session |
| Durable memory | `.rove/memory/MEMORY.md` + `topics/*.md` | Recalled by lexical relevance |

Session memory is written by a post-run hook when a run completes with `TerminationReason::Final`.

Durable memory is managed by tools:

- `save_memory`
- `update_memory_index`
- `read_memory_topic`

`save_memory` rejects unsafe topic names, likely secrets, and transient one-off content before writing.

Current caveat: config exposes `memory.session_dir` and `memory.durable_dir`, but runtime loading and memory tools currently derive memory storage from `workspace.state_dir/memory`.

Relevant code:

- `src/memory/layered.rs`
- `src/memory/session.rs`
- `src/memory/durable.rs`
- `src/hooks/session_memory.rs`
- `src/tools/memory.rs`

## 17. MCP

MCP integration registers remote server tools into the local `ToolRegistry`.

Config path:

```text
.rove/mcp_servers.json
```

Example config:

- `docs/examples/mcp_servers.json`

Supported transports:

- `stdio`;
- `sse`.

For stdio, rove spawns the configured command, sends JSON-RPC messages over stdin, reads stdout lines, initializes the MCP session, calls `tools/list`, and registers each returned tool as:

```text
mcp__<sanitized_server_name>__<remote_tool_name>
```

The proxy maps MCP annotations into local tool metadata:

- `destructiveHint` -> `destructive`;
- `readOnlyHint` -> `parallel_safe` when not destructive.

Current test coverage uses a Python mock stdio server. Real third-party MCP servers should be smoke-tested separately when their schemas or protocol behavior matter.

Relevant code:

- `src/tools/mcp_proxy.rs`
- `tests/mcp.rs`
- `tests/fixtures/mcp_mock_server.py`

## 18. RAG

RAG is optional and gated behind the `rag` feature. Default builds expose stub `retrieve_code` and `retrieve_docs` tools that explain how to enable the feature.

Feature-enabled RAG includes:

- deterministic and OpenAI-compatible embedders;
- a routed embedder foundation that reuses `ModelHealthStore` for production embedding providers;
- staged ingestion;
- fixed, Markdown-aware, and lightweight code-aware chunking;
- LanceDB storage;
- manifest fallback retrieval;
- vector, lexical, and path-scoped channels;
- dedupe and score normalization;
- retrieval eval reports;
- prompt formatting service.

Main artifact paths:

```text
.rove/rag.lancedb
.rove/rag_manifest.json
.rove/rag_index_log.jsonl
.rove/rag_eval/<run_id>.json
```

Useful commands:

```powershell
cargo run --features rag --bin rove-index -- --deterministic -C .
cargo test --features rag --test cli_index deterministic_index_run_writes_manifest -- --exact
```

The CLI uses deterministic embeddings if requested or if no provider API key is configured. Retrieval tools currently use deterministic embeddings. Remote rerank is intentionally not wired in; `NoopRerankPostProcessor` remains the local deterministic fallback until a routed rerank client is introduced.

Relevant code:

- `src/tools/rag/mod.rs`
- `src/tools/rag/index.rs`
- `src/tools/rag/ingest/*`
- `src/tools/rag/retrieve/*`
- `src/tools/rag/eval.rs`
- `src/bin/rove-index.rs`
- `src/interfaces/cli/index.rs`

## 19. API Security

API security is middleware around all routes.

Implemented controls:

- bearer token auth when `api.token_auth` is configured;
- CORS origin allowlist;
- per-process rate limiting;
- rejection of remote bind without token unless explicitly marked unsafe.

Limitations:

- no multi-user identity;
- no distributed rate limiting;
- no browser auth flow;
- the Web client currently does not attach an Authorization header, so token-authenticated API usage needs corresponding Web client work.

Relevant code:

- `src/config.rs`
- `src/interfaces/api/security.rs`
- `tests/api.rs`

## 20. Testing And Verification

Default Rust checks:

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

RAG feature checks:

```powershell
cargo check --features rag --bin rove-index
cargo clippy --all-targets --features rag -- -D warnings
cargo test --features rag
```

Web checks:

```powershell
cd web-ui
npm test
npm run typecheck
npm run build
```

Useful focused tests:

```powershell
cargo test --test api
cargo test --test e2e
cargo test --test mcp
cargo test --features rag --test rag
```

CI is split:

- `.github/workflows/ci.yml` runs default Rust and Web checks.
- `.github/workflows/rag-ci.yml` runs RAG feature checks and index smoke coverage.

## 21. Common Maintenance Tasks

When adding a new tool:

1. Implement `Tool`.
2. Define schema with `destructive` and `parallel_safe`.
3. Register it in CLI and API engine construction.
4. Add executor or e2e coverage.
5. If Web needs to render the result specially, update Web state/types.

When adding a new event:

1. Add a `StreamEvent` variant.
2. Add `event_name`.
3. Update CLI rendering.
4. Update API persistence/SSE assumptions if needed.
5. Update `RunArtifactRecorder` if artifacts should change.
6. Update `web-ui/lib/rove-types.ts` and reducer handling.
7. Add at least one integration test.

When changing run identity or resume:

1. Check `RunRequest`, `RunHandle`, `StateStore::start_run`, and `RunArtifactRecorder`.
2. Check CLI resume path.
3. Check API job creation and persisted replay.
4. Check `.rove/runs/<run_id>/task_state.json` compatibility.

When changing provider tool-use:

1. Update provider parser tests.
2. Update `ModelEvent` normalization.
3. Check engine native tool-use handling in planned and unplanned branches.
4. Check structured history round-trip tests.

## 22. Known Gaps And Risks

These are implementation-level issues to keep in mind before extending the system.

1. RAG paths are hard-coded under `.rove`.
   RAG does not currently consume configurable state paths.

2. `src/core/engine.rs` is too large.
   Planned and unplanned loops duplicate model-turn and tool-call logic. Prefer extracting model turn assembly, approval handling, tool batch execution, and planner-step handling before adding more behavior.

3. Tool pipeline lacks a diff/write-set layer.
   The executor has hooks and approval, but does not yet compute or persist file diffs as a first-class stage.

4. API security and Web auth are not integrated.
   Token auth works in API middleware, but the Web client has no token configuration/header path.

5. MCP coverage is mostly mocked.
   The stdio proxy is covered by a mock server. Real servers such as GitHub/filesystem/postgres should be verified with smoke tests before claiming broad compatibility.

6. Model-generated compaction summaries are not implemented.
    Current checkpoint summaries are deterministic and artifact-based.

## 23. Current Verification Baseline

As of 2026-05-28, the following checks were run locally and passed:

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo check --features rag --bin rove-index
cargo test --features rag
cd web-ui; npm test
cd web-ui; npm run typecheck
cd web-ui; npm run build
```

`cargo test --features rag` took longer because it waited on Cargo artifact locks during the local run, then completed successfully when rerun by itself.
