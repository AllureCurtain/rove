# Subsystem Design

## Configuration

Configuration is typed in `src/config.rs` and grouped by runtime, provider, tool, memory, state, API, web, routing, and RAG.

Merge order:

```text
defaults < .rove/config.toml < environment < CLI/API overrides
```

Validation currently covers provider names, model values, fallback providers, routing thresholds and retry/backoff fields, compaction thresholds, token budgets, RAG timeout, SQLite timeout, memory recall limit, API remote-bind safety, and workspace-relative paths. `rove dump-config` prints effective config with secrets redacted, including provider and RAG key presence flags, plus resolved path fields.

## Workspace

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

`StateStore` coordinates file artifacts and the SQLite `StateIndex`.

Files:

- `trace.jsonl` records append-only runtime events.
- `task_state.json` stores resumable task state and prompt checkpoint.
- `report.json` stores final status, output, and identity metadata.

SQLite:

- stores sessions, jobs, runs, events, reports, task state metadata, pending approval/input tables, and replay offsets;
- uses schema migrations, foreign keys, WAL, `synchronous=NORMAL`, and a bounded busy timeout;
- exposes async helpers through `spawn_blocking` where API handlers need indexed reads;
- supports explicit `rove state repair` and `rove state cleanup` maintenance commands.

Repair treats trace files as the append-only event source and SQLite as a rebuildable index. `rove state repair` imports task state snapshots, report artifacts, trace events, and event offsets; corrupted trace lines are reported and skipped.

## Context And Compaction

`ContextManager` supports token-aware prompt construction with soft, hard, and reserved budgets. Prompt order is:

```text
system -> durable memory -> session memory -> compact summary -> recent history tail -> current user message
```

`TaskState` can include a `PromptCheckpoint` with summary, preserved tail, plan pointer, memory pointers, last step, last event seq, token estimate, and compacted message count. The event sequence matches the SQLite high-water mark for the run. Resume prefers this checkpoint over replaying full audit history.

Default compaction is deterministic and artifact-based. Optional model-generated compaction can be enabled through `runtime.model_compaction_enabled`; when old history is dropped from the active prompt, the runtime asks the current model to produce a resume summary using prompt version `rove.compaction.v1`. Failures do not block the run: rove writes a deterministic fallback summary, records degraded metadata and the last error, and opens a circuit after `runtime.compaction_failure_threshold` consecutive failures.

## Provider And Routing

The model boundary is `ModelClient`, which streams normalized `ModelEvent` values. Raw provider thinking deltas are not exposed to interfaces; the engine converts model-side progress into safe `model_status` stream events. Native providers are peers:

- OpenAI-compatible
- Anthropic
- Ollama
- Fake

Fallback can be configured as:

- `provider.fallback_models`: model names using the primary provider;
- `provider.fallback_providers`: explicit provider/model/base/key records.

Native provider tool-use and JSON text action parsing are both supported. Native tool-use is preferred for real providers because it preserves provider IDs through `Message.tool_calls` and `tool_call_id` history. The JSON text path remains for fake and compatibility scenarios and is used only when a model turn emitted no native tool calls. Planned and unplanned execution share this conversion in `src/core/model_turn.rs`.

`RoutingModelClient` can fall back before user-visible content or committed tool-use begins. It tracks provider health with a failure threshold and cooldown. For each routed candidate, `routing.retry_max_attempts`, `routing.retry_backoff_base_ms`, and `routing.retry_backoff_max_ms` control retry behavior for retryable pre-commit failures; rate-limit `retry-after` values are honored directly. Auth and context-length errors are not retried, and once text or native tool-use has committed, no retry or fallback is attempted.

## Tool Orchestration

Tools are registered in `ToolRegistry` and executed through `Executor`. Tool schemas include `destructive` and `parallel_safe` flags.
CLI and API assemble tools through the same runtime registry builder, which registers built-ins and then loads configured MCP tools.

MCP stdio transport is bounded by per-server policy. Initialize, list, and call requests time out; stderr is captured up to the configured diagnostic limit; JSON-RPC errors are mapped to structured tool execution failures; and child processes are killed when their client is dropped. `cargo test --test mcp` covers mock stdio registration, timeout/error/cleanup behavior, and includes an opt-in real filesystem MCP smoke test gated by `ROVE_MCP_FILESYSTEM_SMOKE=1`.

Batch execution rules:

- multiple non-destructive, parallel-safe calls may run concurrently;
- destructive, unknown, shell, write, request-input, and memory-write style calls serialize through the approval and execution boundary;
- conversation history and trace events are written back in model call order after a batch completes.

Approval policy is `ask`, `auto`, or `never`. The CLI uses stdin for approvals; the API exposes pending approvals through `/jobs/{job_id}/approvals/{call_id}`.

API approval/input restart behavior uses Policy A. Pending records are persisted while live, but the in-memory answer channels are not reconstructed after restart. Startup marks stale running jobs and pending approval/input rows `interrupted`, and resume creates a new run from the last task snapshot.

## Memory

The memory model has three layers:

- working memory: in-run prompt messages built by the engine;
- session memory: `memory.session_dir/<session_id>.md`, written by a post-run hook and used on resume;
- durable memory: `memory.durable_dir/MEMORY.md` plus `topics/*.md`, managed through memory tools.

Durable recall is bounded by `memory.recall_limit` and query relevance. The `save_memory` tool rejects unsafe topic names, obvious secrets, and transient one-off content before writing durable files. CLI and API runs use the same resolved memory paths from config, and session summaries are deterministic markdown with goal, status, output, completed plan steps, tools used, and write-set metadata when available.

## API And Security

The API routes are:

- `POST /jobs`
- `GET /jobs/{job_id}/state`
- `GET /jobs/{job_id}/events`
- `POST /jobs/{job_id}/cancel`
- `POST /jobs/{job_id}/approvals/{call_id}`
- `POST /jobs/{job_id}/inputs/{input_id}`

The API default is local-only binding. Config supports token auth, CORS origin allowlists, rate limits, and an explicit unsafe remote-without-auth override. Token auth, CORS enforcement, and rate limiting are implemented as API middleware.

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
- a `RagPromptService` formatting boundary for retrieved evidence.

RAG artifacts resolve under the configured `state.state_dir`; the default remains `.rove/rag.lancedb`, `.rove/rag_manifest.json`, `.rove/rag_index_log.jsonl`, and `.rove/rag_eval/`. Default builds expose stub `retrieve_code` and `retrieve_docs` tools with disabled capability metadata and JSON output explaining the feature requirement. Feature-enabled builds expose enabled capability metadata. Remote rerank config fields exist, but active rerank execution remains local/noop until a routed rerank provider is added.

## Web

`web-ui/` is a standalone Next.js app. Browser code talks to `/api/*`; a server-side Next.js route proxies requests to `ROVE_API_BASE` or `http://127.0.0.1:8787`. When `ROVE_API_TOKEN` is set on the Next.js server, the proxy injects `Authorization: Bearer <token>` upstream and preserves SSE response bodies for `EventSource`.

The web verification surface is:

```bash
npm test
npm run typecheck
npm run build
```

Browser-level checks are available separately:

```bash
npm run test:e2e
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
