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

## 2. Workspaces

`Workspace::detect` is the first runtime boundary. It canonicalizes the starting directory, walks upward for `.git`, and returns either:

- `WorkspaceKind::Repo` with the nearest git root as `workspace.root`;
- `WorkspaceKind::Folder` with the starting directory as `workspace.root`.

The default state directory is `workspace.root/.rove`. Config can override `state.state_dir`, `state.sqlite_path`, `memory.session_dir`, and `memory.durable_dir`. Core state, memory tools, and RAG artifacts use the resolved config paths.

`WorkspaceKind::Task` is an explicit standalone workspace. It is created under
a task base directory and does not require the shell or API server to start
inside an existing project. The task name must be a single path component; path
traversal and absolute names are rejected. After creation, config is rebased to
the task root so defaults resolve to:

```text
<task-base>/<task-name>/
  .rove/
    state.sqlite
    runs/
    memory/
```

CLI runs create or reuse a task workspace with:

```powershell
cargo run -- --task-workspace invoice-check --task-base .rove/tasks --model fake "review the files in this task"
```

If `--task-base` is omitted, the CLI uses `<configured state_dir>/tasks` from
the initially detected config context.

API jobs can request a task workspace per job:

```json
{
  "message": "review the uploaded task files",
  "model": "fake",
  "workspace": {
    "kind": "task",
    "name": "invoice-check",
    "base": "D:/rove-tasks"
  }
}
```

If `base` is omitted, the API uses `<configured state_dir>/tasks` from the
server workspace config. Each task workspace gets its own resolved state store,
run artifacts, filesystem tool boundary, shell working directory, session
memory, and durable memory paths.

Task workspace lifecycle:

1. Create or reuse the named workspace through CLI `--task-workspace` or API
   `workspace.kind = "task"`.
2. Put task inputs under the task root or let tools create files there.
3. Resume, inspect, repair, or clean state from the same task workspace context.
4. When the task is no longer needed, delete the task workspace directory. This
   removes the task files, `.rove` state, run artifacts, and default memory for
   that isolated task.

Browser and Desktop workspaces are documented future designs only:
`docs/runtime/browser-workspace-spec.md` and
`docs/runtime/desktop-workspace-spec.md`. The runtime intentionally has no
`Browser` or `Desktop` workspace enum stubs yet.

Relevant code:

- `src/core/workspace.rs`
- `src/config.rs`

## 3. Configuration

`AppConfig::load` merges configuration in this order:

```text
defaults < .rove/config.toml < environment < CLI/API overrides
```

The config is grouped by runtime, provider, tool, memory, state, API, web, routing, and RAG. `dump-config` prints the effective config, source summary, resolved paths, and redacted secret presence flags.

Common paths and defaults:

| Config | Default |
|---|---|
| `runtime.system_prompt_path` | `prompts/system.md` |
| `runtime.planner_prompt_path` | `prompts/planner.md` |
| `runtime.model_compaction_enabled` | `false` |
| `runtime.compaction_failure_threshold` | `3` |
| `state.state_dir` | `.rove` |
| `state.sqlite_path` | `.rove/state.sqlite` |
| `tool.mcp_config_path` | `.rove/mcp_servers.json` |
| `tool.shell.timeout_ms` | `30000` |
| `tool.shell.max_output_bytes` | `65536` |
| `tool.shell.inherit_environment` | `true` |
| `tool.shell.denylist` | `[]` |
| `memory.session_dir` | `.rove/memory/sessions` |
| `memory.durable_dir` | `.rove/memory` |
| `routing.failure_threshold` | `3` |
| `routing.open_cooldown_ms` | `30000` |
| `routing.retry_max_attempts` | `1` |
| `routing.retry_backoff_base_ms` | `250` |
| `routing.retry_backoff_max_ms` | `5000` |
| `api.bind_addr` | `127.0.0.1:8787` |
| `rag.deterministic` | `true` |
| `rag.embedding_provider` | `deterministic` |
| `rag.embedding_model` | `deterministic-64` |
| `rag.embedding_api_base` | `https://api.openai.com/v1` |
| `rag.timeout_ms` | `30000` |
| `rag.fallback_to_deterministic` | `true` |

Remote API binding is rejected unless token auth is configured or `api.unsafe_remote_without_auth = true` is set.

Useful commands:

```powershell
cargo run -- dump-config
```

Relevant code:

- `src/config.rs`
- `src/interfaces/cli/config.rs`

## 4. CLI Startup Path

The CLI binary starts in a synchronous `main()` and only creates a Tokio runtime
for commands that need async work. True sync fast paths are handled before
runtime construction.

High-level flow in `src/main.rs`:

1. Parse `Args` from `src/interfaces/cli/args.rs`.
2. If `Args::is_sync_fast_path()` is true, run it without creating a Tokio runtime:
   - `dump-config`
3. Create a Tokio runtime for async commands and normal runs.
4. Run async maintenance subcommands:
   - `index`
   - `sessions`
   - `state repair`
   - `state cleanup`
5. Detect the starting Folder/Repo workspace.
6. Load `AppConfig`.
7. If `--task-workspace` is set, create or reuse that Task workspace and rebase
   config paths to the task root.
8. Construct the model client.
9. Register the shared runtime tool registry, including configured MCP tools.
10. Build `ContextManager`.
11. Build `Engine`.
12. Create `StateStore` and `RunHandle`.
13. Resolve optional CLI resume state.
14. If a message argument is present, run `run_oneshot_with_cancel`.
15. If no message and no subcommand are present, enter the line-oriented REPL.

Current one-shot smoke command:

```powershell
cargo run -- --model fake "echo hello from rove"
```

`Cargo.toml` sets `default-run = "rove"`, so plain `cargo run -- ...` uses the CLI binary.

Running `rove` with no task enters the REPL in the current terminal instead of
printing help or opening a separate window. The REPL prompt is:

```text
rove>
```

Supported slash commands are:

| Command | Purpose |
|---|---|
| `/help` | Print REPL commands. |
| `/exit`, `/quit` | Exit the REPL. |
| `/clear` | Clear the terminal screen. |
| `/sessions` | List resumable task states from the active workspace. |
| `/resume latest` | Load the newest task snapshot as the active resume state. |
| `/resume <run_id>` | Load a specific task snapshot as the active resume state. |

Normal text input runs a new engine run. After a successful non-cancelled run,
the REPL loads the latest task snapshot for follow-up turns, preserving the
session id and reusing the previous job id while creating a new run id. Ctrl+C
while the prompt is idle returns to the prompt; Ctrl+C while a run is active
cancels that run and keeps the REPL process alive.

Relevant code:

- `src/main.rs`
- `src/interfaces/cli/args.rs`
- `src/interfaces/cli/oneshot.rs`
- `src/interfaces/cli/repl.rs`
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

`POST /jobs` accepts `message`, optional `model`, `max_steps`, `approval`,
optional `resume`, and optional `workspace`. The only per-job workspace kind
today is `{"kind":"task","name":"...","base":"..."}`; Folder/Repo jobs use
the API server workspace.
`resume` follows the CLI semantics: omit it for a fresh session/job, use `"latest"` for the newest task snapshot, or pass a run id to load that exact snapshot. A resumed API job keeps the loaded `session_id` and `job_id`, creates a new `run_id`, and passes the loaded `TaskState` into `RunRequest` and artifact recording.

After restart, historical job state and SSE events can be read from SQLite. Pending approvals and pending inputs follow Policy A: requests are persisted for audit while live, but their channels are not reconstructed after process restart. API startup marks stale running jobs and pending approval/input rows as `interrupted`; `/jobs/{job_id}/state` then shows no answerable pending approval/input lists for that historical job. Resuming latest from the interrupted task snapshot starts a new run with a new `run_id`.

Relevant code:

- `src/bin/rove-api.rs`
- `src/interfaces/api/mod.rs`
- `src/interfaces/api/security.rs`

## 6. Web Workbench Path

`web-ui/` is a standalone Next.js app. Browser code talks to relative `/api/*`
URLs. A Next.js app route proxies those requests to the Rust API:

```text
/api/* -> ROVE_API_BASE or http://127.0.0.1:8787
```

If `ROVE_API_TOKEN` is set in the Next.js server environment, the proxy injects
`Authorization: Bearer <token>` into upstream requests. The token is not exposed
through `NEXT_PUBLIC_*` variables or browser JavaScript. SSE job streams are
proxied by returning the upstream response body directly, so `EventSource` keeps
using `/api/jobs/{job_id}/events` without custom browser headers.

The main component:

1. Creates a job with `POST /api/jobs`.
2. Opens an `EventSource` for `/api/jobs/{job_id}/events`.
3. Applies streamed events through `workbenchReducer`.
4. Calls approval and input endpoints when user action is required.
5. Fetches job state on stream errors to resync.

Relevant code:

- `web-ui/components/rove-workbench.tsx`
- `web-ui/app/api/[...path]/route.ts`
- `web-ui/lib/rove-api-proxy.ts`
- `web-ui/lib/rove-client.ts`
- `web-ui/lib/rove-state.ts`
- `web-ui/lib/rove-types.ts`

Current Web checks:

```powershell
cd web-ui
pnpm test
pnpm typecheck
pnpm build
```

Browser-level Playwright tests live under `web-ui/tests/e2e` and run through
`pnpm test:e2e`. They are separate from the default fast Web checks so the
unit/type/build loop stays lightweight.

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
- `model_status`
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
- `prompt_compacted`
- `run_completed`

The API serializes these events as SSE using `StreamEvent::event_name()`. The trace writer serializes the same events to `trace.jsonl` and indexes them in SQLite with sequence numbers.

`model_status` is the safe progress surface for model-side work. It can say
that the model is thinking, has selected a tool, or that the run is waiting for
approval. It must not expose raw provider `ThinkingDelta` or hidden reasoning
text.

Adding a new event requires checking:

- CLI rendering in `src/interfaces/cli/oneshot.rs`
- API SSE/event persistence in `src/interfaces/api/mod.rs` and `src/state/index.rs`
- Web types and reducer in `web-ui/lib/rove-types.ts` and `web-ui/lib/rove-state.ts`
- artifact recording in `src/state/artifacts.rs` if it affects resume/report state

Relevant code:

- `src/core/events.rs`
- `src/state/trace.rs`

## 9. Engine Execution Flow

`Engine` owns the model client, tool registry, context manager, workspace, approval policy, hooks, resolved memory paths, planner prompt, and optional interface providers for approval/input. `Engine` is the public orchestration shell; model turns, tool turns, and planned/unplanned run loops live in focused modules.

The high-level run flow:

1. Emit `RunStarted`.
2. Build history from resume checkpoint or full resume state.
3. Load durable/session memory into working prompt memory.
4. If planning is enabled:
   - draft a new `TaskPlan` with the configured planner prompt, or resume a saved plan;
   - emit `PlanCreated` for initial plans and replacement plans;
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

The planned and unplanned paths share model-turn and tool-turn helpers. If you are changing model streaming, native tool-use conversion, approval, batch execution, or history mutation, start in `model_turn.rs` or `tool_turn.rs`; plan-specific code should only wrap those helpers with plan-step lifecycle events.

Plan mutation semantics:

- Step IDs are stable within one `TaskPlan`.
- Replanning replaces the active plan with a new revision emitted as `PlanCreated`.
- Completed work is preserved as event history and task-state history evidence instead of being blindly copied into the replacement plan.
- Resume prefers the checkpoint plan when present, then the task-state plan. It resumes at `current_step` and does not repeat completed steps unless the saved plan explicitly marks them pending.

Relevant code:

- `src/core/engine.rs`
- `src/core/model_turn.rs`
- `src/core/tool_turn.rs`
- `src/core/run_loop.rs`
- `src/core/plan_loop.rs`
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

Checkpoint compaction has two paths:

- default deterministic compaction, used when model compaction is disabled or as fallback;
- optional model-generated compaction when `runtime.model_compaction_enabled = true`.

When automatic compaction is needed and old history has been dropped from the active prompt, the engine attempts a model summary behind prompt version `rove.compaction.v1`. A successful model summary emits `prompt_compacted` and records `mode = "model_generated"` with model and source-message metadata. If summary generation fails, the run continues with a deterministic fallback summary, degraded/circuit metadata, and the last error. After `runtime.compaction_failure_threshold` consecutive failures, model compaction is circuit-opened for that runtime and deterministic behavior remains available through normal checkpointing.

`RunArtifactRecorder` writes `PromptCheckpoint` with:

- optional summary;
- preserved tail messages;
- current plan;
- memory pointers;
- last step;
- last event sequence matching the SQLite event high-water mark;
- token estimate;
- compacted message count;
- compaction metadata, including mode, degraded state, model, prompt version, source message count, and last error when present.

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

The engine does not forward raw `ThinkingDelta` text to interfaces. It emits
safe `model_status` progress events around model turns and selected tool-use
blocks instead.

Native providers:

| Provider | File |
|---|---|
| OpenAI-compatible | `src/models/openai.rs` |
| Anthropic | `src/models/anthropic.rs` |
| Ollama | `src/models/ollama.rs` |
| Fake | `src/models/fake.rs` |

Provider-native tool use is the preferred path for real providers. Provider adapters emit `ToolUseStart` and `ToolUseDone`, `src/core/model_turn.rs` converts those into `ToolCallAction` values, and `LlmMessage.tool_calls` plus `tool_call_id` preserve structured history for provider replay. OpenAI-compatible, Anthropic, and Ollama formatters replay that history in their native request shapes.

The JSON text action path remains for compatibility and fake-model tests. It is used only when no native tool calls were emitted, flows through `parse_action`, and produces no provider-native `tool_use_id`. Planned and unplanned loops both call the same `run_model_turn` helper, whose `build_action_from_model_output` boundary chooses native tool calls before text fallback.

`RoutingModelClient` wraps a primary model plus fallback models/providers. It can fall back only before committed visible output or committed tool-use. Provider target identity is provider plus endpoint plus model, exposed as `ModelClientId`, so two providers using the same model name do not share a health bucket.

Each routed provider candidate is attempted up to `routing.retry_max_attempts` before moving to fallback. Retryable request failures, stream interruptions, and rate limits before commit use exponential backoff from `routing.retry_backoff_base_ms` capped by `routing.retry_backoff_max_ms`; rate-limit `retry-after` values override the computed delay. Authentication and context-length errors are never retried, though another fallback candidate can still be tried if no output or tool-use has committed. After committed text or committed native tool-use begins, later stream errors are returned directly with no retry and no fallback.

`src/models/health.rs` owns `ModelHealthStore`, `HealthConfig`, and circuit state. CLI-created routed clients keep private health state configured from `routing.failure_threshold` and `routing.open_cooldown_ms`. API state creates one process-shared `ModelHealthStore` and injects it into routed model clients so API jobs share circuit breaker decisions across runs in the same process.

First-packet routing decisions are emitted through `tracing`: candidate start, skipped open circuit, committed first event, no content, timeout, error-before-commit, retry scheduling, and candidate exhaustion. These are observability records only; they do not add user-facing `StreamEvent` variants.

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

CLI and API construct runtime tools through the shared async
`runtime_tool_registry(&Workspace, ShellPolicy, mcp_config_path)` builder. That
builder registers built-ins through `default_tool_registry_with_shell_policy`
and then loads configured MCP tools. Root-bound tools receive the workspace root
at construction. Memory tools are context-bound and derive their paths from
`ToolContext.memory_paths`.

Tool schemas include:

- `destructive`: requires approval unless policy allows it;
- `parallel_safe`: allows concurrent batch execution if every call is non-destructive and safe.
- optional `capability`: lets interfaces distinguish enabled tools from feature-gated disabled stubs.

The executor pipeline is currently:

```text
schema lookup -> argument validation -> pre-tool hooks -> permission -> execute -> result wrapping with mutations -> post-tool hooks
```

Argument validation supports the JSON Schema subset used by built-in tools: object, array, string, number, integer, boolean, and null type checks; required fields; enum values; nested properties; array `items`, `minItems`, and `maxItems`; numeric `minimum` and `maximum`; string `minLength` and `maxLength`; and `additionalProperties: false`. Validation failures preserve `ToolError::InvalidArgs` and happen before tool execution.

Filesystem tools resolve paths through `src/core/boundary.rs`. Reads canonicalize the final target; writes canonicalize existing targets or the nearest existing ancestor for new files. Both paths reject absolute paths, lexical workspace escapes, and symlink/reparse-point escapes that resolve outside the workspace.

`fs_write` returns structured mutation metadata for deterministic file writes. The metadata includes path, operation type, and a textual diff; it is exposed on `ToolCallCompleted.result.mutations` and persisted to `report.json` as `tool_mutations`. Shell commands are bounded by policy and return structured stdout/stderr/exit metadata, but shell write-sets are intentionally not inferred or snapshotted.

Shell policy comes from `tool.shell`: timeout, max output bytes per stream, environment inheritance, and a denylist. The shell working directory is fixed to the workspace root. Empty commands, NUL bytes, denied substrings, timeouts, and output truncation are handled before unbounded history growth.

Tool-call parallelism is conservative and batch-scoped. When a model turn
returns multiple tool calls at once, the runtime runs them concurrently only if
every call is non-destructive and its schema is marked `parallel_safe`. Results
are still written back in model call order. Calls that depend on earlier tool
results naturally happen in later model turns and therefore run serially. The
runtime does not currently infer a general dependency DAG between arbitrary tool
calls.

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

Pending approval/input answer channels are live-only. API rows are persisted while live for audit and state display, but answerable channels are not reconstructed after process restart.

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

`trace.jsonl` is append-only event history and the source used to rebuild event rows during repair. Every line is one serialized `StreamEvent`.

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
- `rove state repair` imports task state artifacts, report artifacts, and trace events; corrupted trace lines are counted and skipped.
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
| Session memory | `memory.session_dir/<session_id>.md` | Loaded on resume / same session |
| Durable memory | `memory.durable_dir/MEMORY.md` + `topics/*.md` | Recalled by lexical relevance |

Session memory is written by a post-run hook when a run completes with `TerminationReason::Final`. The summary is deterministic markdown containing the goal, final status, output excerpt, completed plan steps, tools used, and file write-set metadata when tools report mutations.

Durable memory is managed by tools:

- `save_memory`
- `update_memory_index`
- `read_memory_topic`

`save_memory` rejects unsafe topic names, likely secrets, and transient one-off content before writing.

CLI and API engine assembly pass `AppConfig::memory_paths()` into the runtime, so prompt memory loading, the session-memory post-run hook, and memory tools all use the same resolved `memory.session_dir`, `memory.durable_dir`, and `memory.recall_limit` values. Defaults still resolve to `.rove/memory/sessions` and `.rove/memory`.

Relevant code:

- `src/memory/layered.rs`
- `src/memory/paths.rs`
- `src/memory/session.rs`
- `src/memory/durable.rs`
- `src/hooks/session_memory.rs`
- `src/tools/memory.rs`

## 17. MCP

MCP integration registers remote server tools into the local `ToolRegistry`.
Both CLI and API jobs use the same runtime registry builder, so configured MCP
tools are available through CLI runs, API jobs, and the Web workbench via the
API proxy.

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

Each server can include an optional `policy` object:

```json
{
  "name": "filesystem",
  "transport": "stdio",
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-filesystem", "."],
  "policy": {
    "request_timeout_ms": 30000,
    "stderr_capture_bytes": 16384
  }
}
```

`request_timeout_ms` bounds stdio initialize/list/call requests and SSE HTTP requests. `stderr_capture_bytes` controls how much stdio stderr is retained for timeout and startup diagnostics. MCP JSON-RPC errors are mapped into structured tool execution failures instead of raw protocol blobs. Stdio child processes are killed when their registered client is dropped.

Default test coverage uses Python stdio fixtures for normal registration, timeout, JSON-RPC error mapping, and child cleanup. A real stdio filesystem MCP smoke test is available behind an explicit environment gate:

```powershell
$env:ROVE_MCP_FILESYSTEM_SMOKE = "1"
cargo test --test mcp mcp_official_filesystem_server_smoke_when_enabled -- --exact --nocapture
```

By default that smoke test runs `npx -y @modelcontextprotocol/server-filesystem <temp-dir>` and verifies `read_file`. Override `ROVE_MCP_FILESYSTEM_COMMAND` and `ROVE_MCP_FILESYSTEM_ARGS` when testing a locally installed or pinned server. GitHub or database MCP servers should remain optional and secret-gated when added.

Relevant code:

- `src/tools/mcp_proxy.rs`
- `tests/mcp.rs`
- `tests/fixtures/mcp_mock_server.py`

## 18. RAG

RAG is optional and gated behind the `rag` feature. Default builds expose stub `retrieve_code` and `retrieve_docs` tools with disabled capability metadata and JSON failure output that explains how to enable the feature. Feature-enabled builds mark those schemas with enabled RAG capability metadata.

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

RAG config lives under `[rag]`:

| Config | Default |
|---|---|
| `rag.deterministic` | `true` |
| `rag.embedding_provider` | `deterministic` |
| `rag.embedding_model` | `deterministic-64` |
| `rag.embedding_api_base` | `https://api.openai.com/v1` |
| `rag.embedding_api_key` | empty |
| `rag.rerank_provider` | unset |
| `rag.rerank_model` | unset |
| `rag.rerank_api_key` | unset |
| `rag.timeout_ms` | `30000` |
| `rag.fallback_to_deterministic` | `true` |

`dump-config` prints these fields with API keys redacted as presence flags. Environment overrides use `ROVE_RAG_*` names, such as `ROVE_RAG_DETERMINISTIC`, `ROVE_RAG_EMBEDDING_MODEL`, `ROVE_RAG_EMBEDDING_API_KEY`, and `ROVE_RAG_FALLBACK_TO_DETERMINISTIC`.

Main artifact paths are resolved under the configured `state.state_dir`; the default remains `.rove`:

```text
<state_dir>/rag.lancedb
<state_dir>/rag_manifest.json
<state_dir>/rag_index_log.jsonl
<state_dir>/rag_eval/<run_id>.json
```

Useful commands:

```powershell
cargo run --features rag --bin rove-index -- --deterministic -C .
cargo test --features rag --test cli_index deterministic_index_run_writes_manifest -- --exact
```

The CLI uses deterministic embeddings when requested or when `rag.deterministic = true`. With `rag.deterministic = false`, indexing constructs an OpenAI-compatible embedder from `rag.embedding_api_base`, `rag.embedding_api_key`, and `rag.embedding_model`. If the key is missing and `rag.fallback_to_deterministic = true`, indexing falls back to deterministic embeddings; if fallback is disabled, indexing fails with a config error. Retrieval/eval reports record the embedder and reranker identities. Remote rerank is optional: when `rag.rerank_provider` and `rag.rerank_model` are set, eval retrieval builds a routed reranker using `rag.rerank_api_key` and the RAG embedding API base as the first-pass rerank endpoint base. If rerank is unconfigured, or if fallback is enabled after a provider failure, retrieval uses `rerank-noop` and keeps deterministic local behavior.

Agent tool-time retrieval is intentionally narrower today. The in-agent
`retrieve_code` and `retrieve_docs` tools read the configured state directory
for RAG artifacts, but they still construct deterministic retrieval services
inside the tool. Passing configured embedder/reranker services into
`runtime_tool_registry` is the follow-up direction when tool-time retrieval
needs to use provider-backed embeddings or rerankers.

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
- no browser login/session flow. The local Web workbench supports API bearer
  tokens through its server-side Next.js proxy, not through client-side headers.

These limitations are deployment/product scope for a later phase, not active
runtime gaps for the current local-first target.

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
pnpm test
pnpm typecheck
pnpm build
```

Optional browser E2E checks:

```powershell
cd web-ui
pnpm test:e2e
```

Useful focused tests:

```powershell
cargo test --test api
cargo test --test e2e
cargo test --test mcp
cargo test --features rag --test rag
```

Deterministic local benchmark checks:

```powershell
cargo test --test bench
cargo run --bin rove-bench -- --suite benchmarks/agent-smoke.json --output-dir .rove/bench
```

`rove-bench` reads JSON benchmark task definitions, creates isolated local
workspaces under the output directory, runs scripted fake-model tasks through
the real engine/tool/state paths, and prints a JSON report with pass/fail state
and artifact paths. The default `benchmarks/agent-smoke.json` suite has no
network credential requirement and covers echo/tool smoke, file writing, and
resume context behavior.

The current M0-M6 milestone proof map lives in
`docs/runtime/acceptance-matrix.md`.

CI is split:

- `.github/workflows/ci.yml` runs default Rust and Web checks.
- `.github/workflows/rag-ci.yml` runs RAG feature checks and index smoke coverage.

## 21. Runtime Docs As Source Of Truth

`docs/runtime/` is the current behavior source of truth. Historical design docs
can explain why the runtime exists, but implementation changes should update
the runtime docs first:

- `implementation-status.md` records closed gaps and remaining future scope;
- `implementation-guide.md` records startup paths, runtime flow, artifacts,
  verification, and maintainer procedures;
- `subsystems.md` records subsystem boundaries and intentionally deferred work;
- root `README.md` stays focused on accurate user-facing setup and commands.

Code hygiene is part of the default gate. `src/lib.rs` must not use a global
`#![allow(dead_code)]`; `cargo clippy --all-targets -- -D warnings` is expected
to surface unused stubs. Any local dead-code allowance should carry an inline
reason that explains why the item is intentionally retained.

## 22. Common Maintenance Tasks

### File Size And Module Shape

The historical architecture notes suggested a hard line-count limit for Rust
files. The current runtime docs do not treat that as a binding rule. Prefer
splitting modules when it improves ownership, testability, or reviewability;
keep related code together when a single file makes the behavior easier to
trace. The practical standard is clear responsibility boundaries, not an
absolute line-count threshold.

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
3. Check shared engine native tool-use handling in `src/core/model_turn.rs`.
4. Check structured history round-trip tests.
5. Preserve the native-before-text action conversion in `build_action_from_model_output`.

## 23. Known Gaps And Risks

These are implementation-level issues to keep in mind before extending the system.

1. Agent tool-time RAG retrieval remains deterministic.
   Indexing and eval use configurable embedders and rerankers. The in-agent `retrieve_code` and `retrieve_docs` tools still construct deterministic retrieval directly; passing configured RAG provider services into runtime tool construction remains a follow-up.


## 24. Current Verification Baseline

As of 2026-05-28, the following checks were run locally and passed:

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo check --features rag --bin rove-index
cargo test --features rag
cd web-ui; pnpm test
cd web-ui; pnpm typecheck
cd web-ui; pnpm build
```

`cargo test --features rag` took longer because it waited on Cargo artifact locks during the local run, then completed successfully when rerun by itself.
