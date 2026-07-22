# Subsystem Design

## Configuration

Configuration is typed in `src/config.rs` and grouped by runtime, provider, tool, memory, state, API, web, routing, and RAG.

Merge order:

```text
defaults < .rove/config.toml < environment < CLI/API overrides
```

Validation currently covers provider names, model values, fallback providers, routing thresholds and retry/backoff fields, compaction thresholds, token budgets, RAG timeout, SQLite timeout, memory recall limit, API remote-bind safety, and workspace-relative paths. `rove dump-config` prints effective config with secrets redacted, including provider and RAG key presence flags, plus resolved path fields.

## Workspace

Workspace detection and path-boundary enforcement are implemented in
`runtime/src/workspace.rs` and `runtime/src/boundary.rs`; the root
`rove::core::{workspace,boundary}` paths are compatibility re-exports.

The runtime currently supports three workspace kinds:

- `Folder`: the canonical starting directory when no `.git` ancestor exists.
- `Repo`: the nearest `.git` ancestor, preserving existing repository-scoped behavior.
- `Task`: an isolated standalone directory created under a configured or requested task base.

Task workspaces rebase config resolution to the task root, so default state,
filesystem tools, shell execution, session memory, and durable memory are scoped
under that task directory. CLI runs use `--task-workspace <name>` with optional
`--task-base <path>`. API jobs use a per-job workspace object with
`kind = "task"`, `name`, and optional `base`.

Task cleanup is directory-based: deleting the task workspace directory removes
its local files, `.rove` state, run artifacts, and default memory. Browser and
Desktop workspaces remain future specs only in
`docs/runtime/browser-workspace-spec.md` and
`docs/runtime/desktop-workspace-spec.md`; the runtime has no partial enum or
tool stubs for those kinds.

## State, Job, And Run

`StateStore`, file artifacts, SQLite `StateIndex`, trace/report writers,
repair, cleanup, and resume are implemented in `runtime/src/state/`.
`TaskState`, `PromptCheckpoint`, IDs, lifecycle ledger data, and canonical
`StreamEvent` are owned by the same crate. Transitional `src/state/` and
`src/core/events.rs` modules only re-export these contracts for existing root
callers.

Files:

- `trace.jsonl` records append-only runtime events, including canonical
  `step_result`, `plan_decision`, and `plan_revised` facts for planned
  execution.
- `task_state.json` stores resumable task state, materialized step records,
  plan decisions and revisions, any active step attempt, and the prompt
  checkpoint.
- `report.json` stores final status, output, identity metadata, and the run's
  terminal step records, decisions, and immutable revisions.

SQLite:

- stores sessions, jobs, runs, events, reports, task state metadata, pending approval/input tables, and replay offsets;
- uses schema migrations, foreign keys, WAL, `synchronous=NORMAL`, and a bounded busy timeout;
- exposes async helpers through `spawn_blocking` where API handlers need indexed reads;
- supports explicit `rove state repair` and `rove state cleanup` maintenance commands.

Repair treats trace files as the append-only event source and SQLite as a
rebuildable index. `rove state repair` imports task state snapshots, report
artifacts, trace events (including `step_result`, `plan_decision`, and
`plan_revised`), and event offsets; corrupted trace lines are reported and
skipped. There is no separate mutable lifecycle table or fourth ledger
artifact.

## Context And Compaction

`ContextManager` supports token-aware prompt construction with soft, hard, and reserved budgets. Prompt order is:

```text
system -> durable memory -> session memory -> compact summary -> recent history tail -> current user message
```

`TaskState` can include a `PromptCheckpoint` with summary, preserved tail, plan
pointer, memory pointers, last step, last event seq, token estimate, compacted
message count, and bounded step-ledger metadata. Full step records remain in
the enclosing task-state projection and canonical trace. The event sequence
matches the SQLite high-water mark for the run. Resume prefers this checkpoint
over replaying full audit history.

Default compaction is deterministic and artifact-based. Optional model-generated compaction can be enabled through `runtime.model_compaction_enabled`; when old history is dropped from the active prompt, the runtime first flushes durable-worthy notes from the compacted segment into session memory, then asks the current model to produce a structured resume summary using prompt version `rove.compaction.v2`. Failures do not block the run: rove writes a deterministic structured fallback summary, records degraded metadata and the last error, and opens a circuit after `runtime.compaction_failure_threshold` consecutive failures.

## Provider And Routing

The independent `rove-models` package owns provider-neutral `Message`,
`ToolSchema`, `Usage`, `ModelError`, `ModelClient`, and `ModelEvent` contracts,
plus provider adapters, Fake Model, routing, and health. It has no local project
dependency. The root compatibility package re-exports those types; only
AppConfig-driven construction remains in transitional `src/models/factory.rs`.
Its `ToolSchema` contains only the model-visible name, description, and input
schema. Operational fields live in `rove_core::ToolDescriptor` and are not
included in provider payloads.

The model boundary is `ModelClient`, which streams normalized `ModelEvent` values. Raw provider thinking deltas are not exposed to interfaces; the engine converts model-side progress into safe `model_status` stream events. Native providers are peers:

- OpenAI-compatible
- Anthropic
- Ollama
- Fake

Fallback can be configured as:

- `provider.fallback_models`: model names using the primary provider;
- `provider.fallback_providers`: explicit provider/model/base/key records.

Native provider tool-use and JSON text action parsing are both supported. Native tool-use is preferred for real providers because it preserves provider IDs through `Message.tool_calls` and `tool_call_id` history. The JSON text path remains for fake and compatibility scenarios and is used only when a model turn emitted no native tool calls. Planned, unplanned, and embedded execution share the conversion in `core/src/model_turn.rs`; `src/core/model_turn.rs` translates its `AgentEvent` values to durable `StreamEvent` values.

`RoutingModelClient` can fall back before user-visible content or committed tool-use begins. It tracks provider health with a failure threshold and cooldown. For each routed candidate, `routing.retry_max_attempts`, `routing.retry_backoff_base_ms`, and `routing.retry_backoff_max_ms` control retry behavior for retryable pre-commit failures; rate-limit `retry-after` values are honored directly. Auth and context-length errors are not retried, and once text or native tool-use has committed, no retry or fallback is attempted.

## Tool Orchestration

`rove-core` owns `Tool`, `ToolOutput`, `ToolRegistry`, invocation-scoped
`ToolContext`, argument validation, and `ToolDescriptor`. The descriptor holds
`destructive`, `parallel_safe`, and capability fields while its model-schema
projection omits them. The persistent root runtime executes registered tools
through `Executor`, approval/input handling, hooks, and durable event mapping.
CLI and API assemble tools through the same runtime registry builder, which registers built-ins and then loads configured MCP tools.

Workspace, resolved Memory paths, approval policy, and input providers are
runtime-owned services attached to a tool invocation through a typed extension.
They are not fields on the minimal `rove-core` context, so an embedded custom
Tool needs only call identity and cancellation unless it explicitly opts into
runtime services.

MCP stdio transport is bounded by per-server policy. Initialize, list, and call requests time out; stderr is captured up to the configured diagnostic limit; JSON-RPC errors are mapped to structured tool execution failures; and child processes are killed when their client is dropped. `cargo test --test mcp` covers mock stdio registration, timeout/error/cleanup behavior, and includes an opt-in real filesystem MCP smoke test gated by `ROVE_MCP_FILESYSTEM_SMOKE=1`.

Batch execution rules:

- multiple non-destructive, parallel-safe calls may run concurrently;
- destructive, unknown, shell, write, request-input, and memory-write style calls serialize through the approval and execution boundary;
- conversation history and trace events are written back in model call order after a batch completes.

This is batch-scoped parallelism, not full DAG scheduling. If a tool call needs
the output of a previous call, the model issues it in a later turn and the
runtime runs that sequence serially. The current runtime does not infer hidden
dependencies between arbitrary tool arguments.

Approval policy is `ask`, `auto`, or `never`. The provider contracts and the
task-local request-input registration context are owned by `rove-runtime`; the
root Engine/tool-turn and interface implementations consume them through
compatibility re-exports. The CLI uses stdin for approvals; the API exposes
pending approvals through `/jobs/{job_id}/approvals/{call_id}`.

API approval/input restart behavior uses Policy A. Pending records are
persisted while live, but the in-memory answer channels are not reconstructed
after restart. Startup marks stale running jobs and pending approval/input rows
`interrupted`. An explicit resume still creates a new run from the last task
snapshot, but a planned step that was in flight is not replayed: the new run
emits an `interrupted` `StepRecord` and terminates with an error so an unknown
external side effect cannot be repeated automatically.

## Memory

The memory model has three layers:

- working memory: in-run prompt messages built by the engine;
- session memory: `memory.session_dir/<session_id>.md`, written by a post-run hook and used on resume;
- durable memory: `memory.durable_dir/MEMORY.md` plus `topics/*.md`, managed through memory tools.

Durable recall is bounded by `memory.recall_limit` and query relevance. Recall is CJK-aware, uses smoothed IDF with field weighting, scales by topic confidence, and supports a hard type filter for lower-level callers. The prompt path recalls all memory types. The `save_memory` tool rejects unsafe topic names, obvious secrets, and transient one-off content before writing durable files with `type`, `scope`, `source`, `confidence`, and timestamp metadata. CLI and API runs use the same resolved memory paths from config, and session summaries are deterministic markdown with goal, status, output, completed plan steps, tools used, and write-set metadata when available. Pre-compaction flush blocks are appended to session memory and preserved by final summaries.

## API And Security

The API routes are:

- `POST /jobs`
- `GET /jobs/{job_id}/state`
- `GET /jobs/{job_id}/events`
- `POST /jobs/{job_id}/cancel`
- `POST /jobs/{job_id}/approvals/{call_id}`
- `POST /jobs/{job_id}/inputs/{input_id}`
- `GET /runs`
- `GET /runs/{run_id}/report`
- `POST /providers/test`

The API server also exposes generated documentation:

- `GET /api/openapi.json` returns the OpenAPI specification generated from the route annotations.
- `GET /swagger-ui` serves Swagger UI for browsing the current API surface.

The generated spec documents bearer-token support as `BearerAuth`. Runtime enforcement still follows
`api.token_auth`: business routes require `Authorization: Bearer <token>` when configured, while the
documentation endpoints only expose the static API reference. Provider profiles continue to pass
credential environment variable names through `api_key_env`; raw provider keys are not API fields.

The API default is local-only binding. Config supports token auth, CORS origin allowlists, rate limits, and an explicit unsafe remote-without-auth override. Token auth, CORS enforcement, and rate limiting are implemented as API middleware. Multi-user identity and distributed rate limiting are later deployment/product concerns rather than current runtime requirements.

`POST /providers/test` accepts a provider profile with `name`, `api_base`,
optional `api_key_env`, and optional model id. It checks model inventory for
OpenAI-compatible, Anthropic, and Ollama profiles with server-side credentials
when needed, and returns only redacted key presence and model visibility.
`POST /jobs` may include the same profile to route that single run through an
official API, relay/gateway API, native Anthropic endpoint, local Ollama, or the
fake provider without changing the API process defaults.

## RAG

The RAG implementation is behind `--features rag` and lives under `src/tools/rag/`. It includes:

- deterministic and OpenAI-compatible embedders;
- explicit RAG provider config with deterministic fallback behavior;
- staged ingestion with logging;
- fixed, markdown-aware, and lightweight code-aware chunking;
- LanceDB storage plus manifest fallback;
- vector, lexical, and path-scoped retrieval channels;
- postprocessing for dedupe and score normalization;
- pure retrieval eval reports that record embedder and reranker identity;
- optional routed remote rerank for eval retrieval with `rerank-noop` fallback;
- a `RagPromptService` formatting boundary for retrieved evidence.

RAG artifacts resolve under the configured `state.state_dir`; the default remains `.rove/rag.lancedb`, `.rove/rag_manifest.json`, `.rove/rag_index_log.jsonl`, and `.rove/rag_eval/`. Default builds expose stub `retrieve_code` and `retrieve_docs` tools with disabled capability metadata and JSON output explaining the feature requirement. Feature-enabled builds expose enabled capability metadata. Remote rerank is optional for eval retrieval: when `rag.rerank_provider`, `rag.rerank_model`, and `rag.rerank_api_key` are configured, the routed reranker calls the configured provider endpoint and records the reranker identity in reports; otherwise eval retrieval uses `rerank-noop`.

The in-agent `retrieve_code` and `retrieve_docs` tools currently use deterministic
retrieval services at execution time while reading artifacts from the configured
state directory. Extending runtime tool construction to inject configured
embedder/reranker services is the planned direction when provider-backed
tool-time retrieval is needed.

## Web

`web-ui/` is a standalone Next.js app. Browser code talks to `/api/*`; a server-side Next.js route proxies requests to `ROVE_API_BASE` or `http://127.0.0.1:8787`. When `ROVE_API_TOKEN` is set on the Next.js server, the proxy injects `Authorization: Bearer <token>` upstream and preserves SSE response bodies for `EventSource`.

The workbench exposes a provider selector for runtime default vs.
OpenAI-compatible per-run profiles. For official APIs and relay/gateway APIs,
users enter API base URL, key environment variable name, and model id, then use
the Test action before starting a run. Browser code sends only the key
environment variable name; raw provider keys stay in the Rust API server
environment.

The web verification surface is:

```bash
pnpm test
pnpm typecheck
pnpm build
```

The Web event contract includes plan/revision/attempt identity, `step_result`,
`plan_decision`, and `plan_revised`. The reducer retains records, decisions,
and revisions in deduplicated structured projections while preserving the
compatibility plan-step timeline behavior.

Browser-level checks are available separately:

```bash
pnpm test:e2e
```

## CI

CI is split by dependency weight:

- `.github/workflows/ci.yml`: Rust default fmt/clippy/test and web test/typecheck/build.
- `.github/workflows/rag-ci.yml`: RAG feature clippy, full `--features rag` tests, and `rove-index` feature/smoke coverage.

RAG remains separate so DataFusion/LanceDB dependencies do not slow every default feedback loop.

## Benchmark And Acceptance

`rove-bench` runs deterministic benchmark suites described by JSON files under
`benchmarks/`. The default `benchmarks/agent-smoke.json` suite uses the fake
model, requires no network credentials, and exercises the real engine, tool
registry, state store, trace writer, task snapshot, and report writer. Its
output is JSON with suite/task pass-fail status and artifact paths for each run.

Run it with:

```bash
cargo run --bin rove-bench -- --suite benchmarks/agent-smoke.json --output-dir .rove/bench
```

The milestone proof map is maintained in
`docs/runtime/acceptance-matrix.md`, which ties M0-M6 criteria to concrete
verification commands.
