# Subsystem Design

## Configuration

Configuration is typed in `apps/bootstrap/src/config.rs` and grouped by
runtime, provider, tool, memory, state, API, web, and routing.

Merge order:

```text
defaults < .rove/config.toml < environment < CLI/API overrides
```

Validation covers legacy and named provider selection, profile/fallback
references, endpoints, model and protocol-option bounds, auth/header names,
workspace-bounded secret files, routing thresholds and retry/backoff fields,
compaction thresholds, token budgets, SQLite timeout, memory recall limit, API
remote-bind safety, and workspace-relative paths. `rove dump-config` prints the
effective config with legacy key-presence flags and named-profile secret/header
source summaries; resolved secret values and literal header values are omitted.

## Workspace

Workspace detection and path-boundary enforcement are implemented in
`runtime/src/workspace/root.rs` and `runtime/src/workspace/boundary.rs`.

The runtime currently supports three workspace kinds:

- `Folder`: the canonical starting directory when no `.git` ancestor exists.
- `Repo`: the nearest `.git` ancestor, preserving existing repository-scoped behavior.
- `Task`: an isolated standalone directory created under a configured or requested task base.

Task workspaces rebase config resolution to the task root, so default state,
filesystem tools, shell execution, session memory, and durable memory are scoped
under that task directory. CLI runs use `--task-workspace <name>` with optional
`--task-base <path>`. API jobs use a per-job workspace object with
`kind = "task"`, `name`, and optional `base`.

Folder and Repo product binding (Web M1 F0) uses the same per-job workspace
object with `kind = "folder"` or `"repo"` and an absolute `root`. The API opens
that path as the real execution root (tool boundary + rebased state/memory),
without walking up to a parent git root for Folder, and requiring `.git` at
`root` for Repo. Hard resume continues only when the second turn targets the
same workspace root and durable `task_state`; a resume key that resolves to no
state in that workspace store is rejected with 400 (no silent one-shot
fallback).

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
`StreamEvent` are owned by the same crate.

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

The context builder and both compaction implementations live in
`runtime/src/context/manager.rs` and `runtime/src/context/compaction.rs`. The runtime run and
step loops coordinate when compaction and its durable events occur.

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
`ToolDescriptor`, `Usage`, `ModelError`, `ModelClient`, and `ModelEvent` contracts,
plus provider adapters, Fake Model, routing, and health. It has no local project
dependency. AppConfig-driven construction lives in
`apps/bootstrap/src/factory.rs`.
Its `ToolDescriptor` contains only the model-visible name, description, and input
schema. Operational fields live in `rove_core::ToolDescriptor` and are not
included in provider payloads.

`models/src/provider/` contains validated open wire-protocol IDs, a
duplicate-safe strategy registry, per-stream decoder contracts, bounded
byte-safe SSE/JSONL framing, resolved auth/header redaction wrappers, a shared
bounded HTTP transport, `ProviderClient`, native OpenAI Chat / Responses /
Anthropic Messages / Ollama Chat strategies and decoders, and the opt-in
`external-adapter-v1` process client. Product bootstrap resolves all native
HTTP targets through `ProviderClient`, routes `external-adapter-v1` profiles to
the process client, and keeps Fake as a local deterministic client. Legacy dual
client modules under `models/src/openai.rs` (and siblings) are test-only parity
helpers and are not part of production assembly.
Invalid transport configuration is typed as `ModelError::InvalidConfiguration`
and is not retried or counted as a provider-health failure.

Provider configuration supports an explicit named-profile form:

```toml
[provider]
active = "team-gateway"
fallback_profiles = ["claude"]

[provider.profiles.team-gateway]
provider_type = "openai"
base_url = "https://gateway.example.test/v1"
model = "team/model"
auth = { style = "bearer", secret = { env = "TEAM_GATEWAY_KEY" } }

[provider.profiles.claude]
provider_type = "anthropic"
base_url = "https://api.anthropic.com"
model = "claude-sonnet"
auth = { style = "header", header = "x-api-key", secret = { env = "ANTHROPIC_API_KEY" } }
```

Secret references may use bounded environment variables or UTF-8 files. Files
are limited to the workspace unless `state.allow_external_paths` is enabled.
Known wire protocols work with official APIs, self-hosted endpoints, and
compatible gateways by changing profile data. Applications may inject a
custom in-process `WireProtocolRegistry` through `ModelClientFactory`; unknown
IDs fail explicitly and never fall back to OpenAI behavior.

Provider config is **profiles-only**: `provider.active` plus
`provider.profiles.<name>` with product field `provider_type`. The system maps
`provider_type` to an internal `wire_protocol` (for example `openai` →
`openai-completions`). Flat `provider.name` / `api_base` / `api_key` assembly is
gone. API and Web per-run profiles use the same product types (`openai`,
`openai-responses`, `anthropic`, `ollama`, or `fake`). Official endpoints and
relays share the same type; only `api_base`, key env, and model differ. Display
`name` is optional and defaults from `api_base` (hostname). Requests must not
send a writable `wire_protocol`; responses may echo the mapped id for debugging.
Secrets continue to be passed only as environment variable names, never as raw
key values in browser-visible fields.

The model boundary is `ModelClient`, which streams normalized `ModelEvent` values. Raw provider thinking deltas are not exposed to interfaces; the engine converts model-side progress into safe `model_status` stream events. Native providers are peers:

- OpenAI Completions (`openai` → `openai-completions`)
- OpenAI Responses (`openai-responses`)
- Anthropic Messages (`anthropic` → `anthropic-messages`)
- Ollama (`ollama`)
- Fake (`fake`)
- Opt-in external process adapter (`external-adapter-v1`)

Fallback can be configured as:

- `provider.fallback_models`: model names using the primary provider;
- `provider.fallback_profiles`: named target profiles.

Native provider tool-use and JSON text action parsing are both supported. Native tool-use is preferred for real providers because it preserves provider IDs through `Message.tool_calls` and `tool_call_id` history. The JSON text path remains for fake and compatibility scenarios and is used only when a model turn emitted no native tool calls. Planned, unplanned, and embedded execution share the conversion in `core/src/model_turn.rs`; `runtime/src/engine/model_turn.rs` translates its `AgentEvent` values to durable `StreamEvent` values.

`RoutingModelClient` can fall back before user-visible content or committed tool-use begins. It tracks provider health with a failure threshold and cooldown. For each routed candidate, `routing.retry_max_attempts`, `routing.retry_backoff_base_ms`, and `routing.retry_backoff_max_ms` control retry behavior for retryable pre-commit failures; rate-limit `retry-after` values are honored directly. Auth and context-length errors are not retried, and once text or native tool-use has committed, no retry or fallback is attempted.

## Tool Orchestration

`rove-core` owns `Tool`, `ToolOutput`, `ToolRegistry`, invocation-scoped
`ToolContext`, argument validation, and `ToolDescriptor`. The descriptor holds
`destructive`, `parallel_safe`, and capability fields while its model-schema
projection omits them. Local built-in tool implementations and their typed
invocation adapters live in `runtime/src/tools/`. The tool `Executor`
pipeline, pre/post-tool plus post-run hooks (including session-summary), and the
durable tool-turn coordinator live in `runtime/src/tools/executor.rs`,
`runtime/src/tools/hooks/`, and `runtime/src/engine/tool_turn.rs`. The existing
stdio/legacy-SSE MCP proxy is implemented in `runtime/src/tools/mcp_proxy.rs`.
CLI and API assemble tools through the same product registry builder
(`apps/bootstrap::tool_registry` / `tool_registry_with_mcp`), which registers
runtime built-ins and then loads configured MCP tools.

Workspace, resolved Memory paths, approval policy, and input providers are
runtime-owned services attached to a tool invocation through a typed extension.
They are not fields on the minimal `rove-core` context, so an embedded custom
Tool needs only call identity and cancellation unless it explicitly opts into
runtime services.

MCP stdio transport is bounded by per-server policy. Initialize, list, and call requests time out; stderr is captured up to the configured diagnostic limit; JSON-RPC errors are mapped to structured tool execution failures; and child processes are killed when their client is dropped. `runtime/tests/mcp_contract.rs` and `cargo test --test mcp` cover mock stdio registration, annotation safety, timeout/error/cleanup behavior, and include an opt-in real filesystem MCP smoke test gated by `ROVE_MCP_FILESYSTEM_SMOKE=1`.

Batch execution rules:

- multiple non-destructive, parallel-safe calls may run concurrently;
- destructive, unknown, shell, write, request-input, and memory-write style calls serialize through the approval and execution boundary;
- conversation history and trace events are written back in model call order after a batch completes.

This is batch-scoped parallelism, not full DAG scheduling. If a tool call needs
the output of a previous call, the model issues it in a later turn and the
runtime runs that sequence serially. The current runtime does not infer hidden
dependencies between arbitrary tool arguments.

Approval policy is `ask`, `auto`, or `never`. The provider contracts and the
task-local request-input registration context are owned by `rove-runtime`.
Product shells assemble `runtime::Engine` through `apps/bootstrap` and consume
those contracts directly. The CLI uses stdin for approvals; the API exposes
pending approvals through `/jobs/{job_id}/approvals/{call_id}`.

API approval/input restart behavior uses Policy A. Pending records are
persisted while live, but the in-memory answer channels are not reconstructed
after restart. Startup marks stale running jobs and pending approval/input rows
`interrupted`. An explicit resume still creates a new run from the last task
snapshot, but a planned step that was in flight is not replayed: the new run
emits an `interrupted` `StepRecord` and terminates with an error so an unknown
external side effect cannot be repeated automatically.

## Memory

Memory paths, session storage, durable topic parsing/recall, layered prompt
loading, built-in memory tools, and the session-summary post-run hook live in
`runtime/src/`.

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
- `POST /providers/models`
- `POST /providers/test`

The API server also exposes generated documentation:

- `GET /api/openapi.json` returns the OpenAPI specification generated from the route annotations.
- `GET /swagger-ui` serves Swagger UI for browsing the current API surface.

The generated spec documents bearer-token support as `BearerAuth`. Runtime enforcement still follows
`api.token_auth`: business routes require `Authorization: Bearer <token>` when configured, while the
documentation endpoints only expose the static API reference. Provider profiles continue to pass
credential environment variable names through `api_key_env`; raw provider keys are not API fields.

The API default is local-only binding. Config supports token auth, CORS origin allowlists, rate limits, and an explicit unsafe remote-without-auth override. Token auth, CORS enforcement, and rate limiting are implemented as API middleware. Multi-user identity and distributed rate limiting are later deployment/product concerns rather than current runtime requirements.

Provider profiles use a user-facing **type** (`provider_type`: `openai`,
`openai-responses`, `anthropic`, `ollama`, or `fake`) plus `api_base` and
optional display `name` / `api_key_env`. Official and relay endpoints share the
same type; only base URL, key, and model differ.

- `POST /providers/models` lists available model ids for that endpoint. OpenAI
  and Anthropic families require a server-side key from `api_key_env`; Ollama
  and Fake do not. The response returns `models: string[]` and never raw
  secrets.
- `POST /providers/test` checks whether a selected model is present in that
  inventory (`model_present`) and reports key presence / inventory count. Use
  this after the user picks a model; use `/providers/models` to discover
  options first.
- `POST /jobs` may include the same profile to route that single run through an
  official API, relay/gateway API, native Anthropic endpoint, local Ollama, or
  the fake provider without changing the API process defaults.

## Workspace retrieval

rove does not ship a built-in vector database. Agents retrieve workspace context with filesystem/shell tools and layered session/durable memory. Future semantic retrieval, if any, would be an optional external service and is not implemented.


## Web

`apps/web/` is a standalone Next.js app. Browser code talks to `/api/*`; a server-side Next.js route proxies requests to `ROVE_API_BASE` or `http://127.0.0.1:8787`. When `ROVE_API_TOKEN` is set on the Next.js server, the proxy injects `Authorization: Bearer <token>` upstream and preserves SSE response bodies for `EventSource`.

The workbench exposes a provider selector for runtime default vs.
OpenAI per-run profiles. For official APIs and relay/gateway APIs,
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

Default feedback loops stay free of heavy retrieval dependencies. Workspace
retrieval is tool-based (`read_file` / `search_code` / `run_shell`) plus layered
session/durable file memory; there is no built-in vector database. Prefer
`search_code` for structured code search and reserve `run_shell` for arbitrary
commands.

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
