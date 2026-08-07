# Subsystem Design

## Configuration

Configuration is typed in `apps/bootstrap/src/config.rs` and grouped by
runtime, provider, tool, memory, state, API, web, and routing.

Project Trust is persistent, granular, and fail-closed. It has `unknown`,
`restricted`, `trusted`, and `revoked` states and independently grants project
configuration, workspace instructions, MCP/process definitions, hooks or
extensions, provider/credential selectors, and external paths. Records bind an
exact canonical root and workspace kind to a stable platform identity plus one
digest per capability. A changed executable input invalidates only its matching
capability; a parent grant does not cover a nested repository, and replacement,
symlink, junction, and alias resolution is conservative.

Bootstrap, CLI, API, and runtime all use the same operator-owned SQLite
authority selected by `ROVE_PROJECT_TRUST_STORE` or the platform user-state
directory (`project-trust.sqlite` by default). Product Web sends a server-owned
workspace ID; the API resolves that ID to the canonical root before calling the
same repository. ProductStore schema v11 retains `project_trust_records` only
as a one-way compatibility import source. It is not written by the API and is
not a second live authority. Missing canonical records are imported at API
startup without overwriting an existing canonical decision.

An old JSON authority is read once, validated, renamed to
`project-trust.json.legacy`, and imported into SQLite in one transaction. The
legacy backup is retained for rollback: remove the new SQLite file and restore
the backup only after an operator review. A failed import leaves the backup in
place and never grants trust implicitly.

CLI `--trust-project` and process-level `ROVE_TRUSTED_WORKSPACES` remain exact-
root temporary grants. They grant only the current process and are never
silently converted into durable records. Workspace `.rove/config.toml` and
`.env` must resolve inside the workspace and stay within the bootstrap size
limit. Their values are filtered before merge: provider fields and referenced
secret values require `provider_credentials`, the MCP path requires
`mcp_processes`, external path fields require `external_paths`, and other
configuration requires `project_configuration`. Project `.env` values are held
in a redacted, invocation-scoped map and never mutate the process environment;
operator environment values retain higher precedence. Repository text cannot
grant or widen trust, and trust never replaces per-tool approval.

`rove trust query|grant|deny|revoke` exposes durable CLI operations with
repeated `--capability` selectors and the same stable trust error codes as the
API. `--trust-project` remains a process-only compatibility grant and is never
persisted.

Merge order for an explicitly trusted workspace:

```text
defaults < .rove/config.toml < environment < CLI/API overrides
```

For a restricted or revoked workspace, the project-config layer is reported as
present but deferred, and process environment plus explicit overrides apply
over defaults. `rove dump-config` exposes the non-secret activation source,
identity digest, invalidated/granted capability names, and whether project
config was present or loaded. Product Web Settings exposes explicit grant,
deny, and revoke controls; browser requests send only workspace IDs. Revocation
blocks new activation and cancels matching live API jobs. Each job uses a
bounded trust-store monitor, so a CLI or other-process write to the canonical
operator database is observed without relying on the API decision route.
Product provider digests include stable, sorted ProductStore session/profile
selectors (provider type, endpoint, credential environment name, and model),
never credential values. The existing cancellation path terminates foreground
child work and records the normal canonical cancellation lifecycle; no new
event family was introduced.

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

History limits preserve provider-native assistant tool calls and their matching results as atomic units; an incomplete or orphan native round is excluded from the active provider prompt. Default compaction is deterministic and artifact-based. Optional model-generated compaction can be enabled through `runtime.model_compaction_enabled`; when old history is dropped from the active prompt, the runtime first flushes durable-worthy notes from the compacted segment into session memory, then serializes that segment as JSON in one ordinary user data message and asks the current model to produce a structured resume summary using prompt version `rove.compaction.v3`. Original tool protocol roles are not replayed to the compaction provider, and embedded text is marked as untrusted historical data. Failures do not block the run: rove writes a deterministic structured fallback summary, records degraded metadata and the last error, and opens a circuit after `runtime.compaction_failure_threshold` consecutive failures.

## Provider And Routing

The independent `rove-models` package owns provider-neutral `Message`,
`ModelToolSchema`, `Usage`, `ModelError`, `ModelClient`, and `ModelEvent` contracts,
plus provider adapters, Fake Model, routing, and health. It has no local project
dependency. AppConfig-driven construction lives in
`apps/bootstrap/src/factory.rs`.
Its `ModelToolSchema` contains only the model-visible name, description, and input
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
`destructive`, `parallel_safe`, stable optional `capability_id`, and availability
fields while its model-schema projection omits them. Registration reads the
descriptor once, validates the bounded provider-neutral JSON Schema subset,
and pins the descriptor plus model projection for later lookup and execution.
The registry is lexically ordered; duplicate names, duplicate capability IDs,
invalid schemas, and excessive catalogs fail without overwriting an existing
entry. The compatibility `register` wrapper is for trusted built-ins, while
dynamic catalogs use fallible atomic batch registration. Local built-in tool implementations and their typed
invocation adapters live in `runtime/src/tools/`. The tool `Executor`
pipeline, pre/post-tool plus post-run hooks (including session-summary), and the
durable tool-turn coordinator live in `runtime/src/tools/executor.rs`,
`runtime/src/tools/hooks/`, and `runtime/src/engine/tool_turn.rs`. The existing
stdio/legacy-SSE MCP proxy is implemented in `runtime/src/tools/mcp_proxy.rs`.
CLI and API assemble tools through the same product registry builder. The
config-aware `tool_registry_for_config` always registers runtime built-ins but
loads configured MCP tools only when the exact workspace has explicit project
activation. A restricted workspace never reads or spawns its MCP definitions.

Core validates tool schemas and provider streaming/tool-call capabilities
before invoking `ModelClient::stream`, including for custom clients. Runtime
then derives an immutable `CapabilitySnapshot` from the pinned registry. New
runtime identities and plan revisions carry its stable ID, and Planner receives
a bounded summary that labels metadata as data rather than permission. Tool
policy, approval, workspace, and execution-environment checks remain mandatory
at invocation time.

Workspace, resolved Memory paths, approval policy, and input providers are
runtime-owned services attached to a tool invocation through a typed extension.
They are not fields on the minimal `rove-core` context, so an embedded custom
Tool needs only call identity and cancellation unless it explicitly opts into
runtime services.

`runtime/src/environment.rs` owns the first-wave Execution Environment ports:
`ExecutionEnvironment`, `WorkspaceFileSystem`, `ProcessHost`, redacted
identity/capabilities, local and in-memory adapters, and a bounded observation
store. File read/write, code search, foreground Shell, MCP config reads, and
stdio MCP spawn/cleanup use these ports through `RuntimeToolServices`. The
local adapter owns canonical path enforcement, output bounds, timeouts,
cancellation, kill-and-wait cleanup, and process cwd. The in-memory adapter
supports deterministic parity tests and typed missing-capability failures
before side effects.

The environment workspace digest is the existing redacted
`RuntimeIdentity.workspace_fingerprint`. New runtime identities also persist
optional `execution_environment` and `execution_capabilities` fields containing
the adapter kind, workspace kind/digest, and boolean capabilities. No raw path
is added, and old artifacts without these additive fields remain readable. The
Product runtime endpoint exposes only adapter kind, workspace kind, the digest,
and boolean capability availability. `ObservationStore` provides stable identity,
source/range, byte count, digest/version, truncation, optional artifact
reference, bounded retention, and stale-version rejection. First-wave tools
preserve their existing request/output contracts; ranged reads, exact edits,
background Shell, and other Coding Tool V2 behavior remain unimplemented.

MCP stdio transport is bounded by per-server policy. Initialize, list, and call requests time out; stderr is captured up to the configured diagnostic limit; JSON-RPC errors are mapped to structured tool execution failures; and child processes are killed when their client is dropped. `tests/mcp.rs` and `cargo test -p rove-integration-tests --test mcp` cover mock stdio registration, annotation safety, timeout/error/cleanup behavior, and include an opt-in real filesystem MCP smoke test gated by `ROVE_MCP_FILESYSTEM_SMOKE=1`.

An MCP discovery request accumulates the enabled-server catalog before one
atomic registry commit. Invalid schemas, aliases, or capability bindings leave
the prior registry unchanged. MCP tools receive stable namespaced capability
IDs derived from the configured server and exact remote identity, while local
safety remains conservative (`destructive`, non-parallel). This is catalog
pinning for the current Engine, not live MCP capability refresh.

All MCP transports are byte-bounded by `MAX_MCP_RESPONSE_BYTES` (1 MiB): stdio
JSON lines, legacy SSE endpoint discovery, and SSE JSON responses. HTTP bodies
accumulate in chunks and honor a declared `Content-Length` before reading, so a
hostile or broken server cannot force an unbounded read. Tools with an empty name
are rejected. These conditions classify as protocol mismatch rather than a
generic transport error.

A remote `readOnlyHint` or `destructiveHint` describes intent only and is never a
local policy grant: MCP proxy tools stay destructive and non-parallel locally, so
they still require approval under an `Ask` policy. Product-managed catalogs are
workspace-bounded and store only environment variable *names*; values resolve by
name at spawn time. Catalog reads and writes are lock-guarded, and a corrupt,
locked, symlinked, or non-regular-file catalog fails closed as a typed conflict
instead of degrading to an empty tool list.
Catalog listing and editing remain available for safe inspection in a
restricted workspace, but `probe` returns `project_trust_required` before
environment resolution or process spawn. Job-start responses report the typed
`workspace_activation` state.

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

The default route is the Web product shell with Workspace/Session navigation,
Chat, a collapsible Run Inspector, and a full-page Settings shell. Durable routes
cover `/w/:workspaceId`, `/w/:workspaceId/s/:sessionId`, and
`/settings/:section`; `/` redirects to the safe server-preferred location or an
empty state, and invalid catalog/route combinations fail explicitly.
`/dev/workbench` is an advanced migration escape hatch, not a second primary
entry.

Providers supports runtime default, OpenAI, OpenAI Responses, Anthropic,
Ollama, and Fake per-run profiles. Users provide API base, key environment
variable name, and model id, then use Test/List Models. Browser code sends only
the key environment variable name; raw provider keys stay in the Rust API
server environment.

Web Complete C0 implements the backend product-control plane in
`apps/api/src/product/`: API-global `product.sqlite`, validated
workspace/session/profile/preferences CRUD, exact server-owned
product-session/runtime bindings, single-active-turn claims, and transcript
projection over canonical sequenced runtime events with typed partial reasons.
ProductStore retains safe catalog/settings/mapping state only; canonical
trace/task/report facts remain in each execution workspace. The C0 Web modules
provide strict response validation, a thin product client, and a versioned,
same-origin-locked, replay-safe M1 migration state machine that never uploads
raw keys.

Web Complete C1 wires the default `ProductApp` to those C0 clients. Startup
loads the API-authoritative workspace/session catalog, preferences, and provider
profiles. Session entry reads the canonical transcript and projects messages,
tools, approvals, inputs, and run identity through the shared reducer. Its
ordered presentation index uses run ordinal/event sequence identity, keeps
tool and interaction cards at their canonical position, and deduplicates
replayed event sequences. Handled input prompts remain read-only without
persisting the answer. Partial history and storage failures remain explicit and
retryable instead of becoming an empty conversation.
Product turns include the exact `product_session_id` and omit client `resume`,
so the server resolves the session's own latest runtime binding rather than
workspace-global `latest`.

Only the focused live job owns an `EventSource`. Switching sessions closes that
observation, restores the selected transcript, and reattaches when its durable
status/binding is live; running and attention badges for background sessions are
refreshed by bounded catalog polling. A network-ambiguous `POST /jobs` is not
retried automatically: the shell performs bounded session-binding reads,
attaches an advanced binding when visible, or restores the canonical transcript
and surfaces an explicit uncertain state. Provider list/create/update/delete and active
selection use the API store; raw keys remain outside browser state and requests.
A failed cancel request leaves that focused observation attached, while a
confirmed terminal cancellation closes it normally.

Web Complete C2 adds revision-safe preferences and a durable default approval
policy honored by product jobs, bounded durable-memory and runtime-health
routes, and complete Settings UI. All nine sections now have real capabilities:
theme, provider CRUD/test/models, approval and step defaults, workspace/session
management and safe export, Memory browse/delete, four keyboard shortcuts,
Advanced Benchmark, and runtime/resume health.

The product durable-memory routes require a server-owned `workspace_id`. They
resolve ProductStore's canonical root through the same config rebase used by
product jobs; browser-supplied filesystem paths are not accepted. The Web
Memory surface always requires the rebased durable directory to remain inside
the selected workspace, including when general runtime configuration enables
`state.allow_external_paths`. An external or cross-workspace resolved directory
returns typed `product_memory_conflict`; unknown workspaces and absent topics
return typed 404 errors. DELETE returns 204 for a physically deleted
selected-workspace file, including a valid unindexed topic. If no topic file was
deleted, stale-index-only cleanup and a fully absent topic both return typed
404.

Web Complete C3 wraps server product state in `M1MigrationGate`. Before any
catalog read, it checks legacy browser state and the durable migration receipt;
only `not_needed` or verified `complete` mounts `useServerProductState`.
Pending, rejected, blocked, or superseded outcomes remain explicit and fail
closed, retries preserve the stored idempotency key and exact request body, and
a fresh completion can remap a legacy product route through server-issued IDs.
C3 also completes responsive, focus, keyboard, reduced-motion, theme, and
empty/loading/error/partial/success polish.

The current CDH G1 control surface adds six canonical lifecycle events:
`steer_accepted`, `steer_applied`, `steer_dropped`, `followup_queued`,
`followup_dequeued`, and `followup_abandoned`. A product steer is persisted
first and delivered through the live run's bounded control handle; the runtime
accepts it only at a declared pre-model safe point and records a drop when a
terminal outcome prevents application. Product follow-ups are server-owned,
idempotent queue records. A final turn atomically claims the next queued item
and launches a new exact product-session turn; a non-final or indeterminate
turn abandons it for explicit confirmation or revoke. Startup recovery drains
only idle sessions with safely pending work and does not replay a reserved
side effect.

The Composer exposes Steer and Follow-up modes while a run is active, retains
the Stop action in either mode, and displays the server-backed control queue.
The queue reflects durable status, supports revoke, and offers explicit
confirmation for an abandoned follow-up. It does not synthesize a client-side
follow-up run when the session appears idle.

The current CDH G2 fork surface permits a branch only from an API-verified,
terminal canonical run boundary. `product_session_forks` and its inherited-run
records retain the parent identity, exact terminal sequence, and read-only
source prefix even if the parent session metadata is deleted. A child begins
with fresh runtime SessionId/JobId/RunId values: source TaskState is reused only
to seed bounded history, not as a normal resume relation. Transcript projection
marks the source prefix `inherited` and keeps child events in a separate local
ledger. The product shell exposes Fork only for a completed latest turn and
renders parent/child rows with the persisted fork point; catalog session loading
has a fixed ProductStore collection limit rather than unbounded tree traversal.

CDH G3-G7 are also implemented on `main`. Session model/reasoning/approval/
step-limit configuration uses revision CAS and immutable per-run snapshots;
usage/cost/context inspection preserves explicit unavailable states; product
file browsing, artifacts, image validation, and run/Git diff are bounded and
workspace/session scoped; evidence export renders redacted JSON, offline HTML,
and Markdown from one sanitized value; and the workspace-scoped MCP catalog is
shared by Settings and jobs with secret-name-only persistence, typed probes,
1 MiB transport bounds, and fail-closed corrupt/locked/unsafe configuration.
The exact contract/test map is in `acceptance-matrix.md` under CDH G1-G7.

The web verification surface is:

```bash
pnpm test
pnpm typecheck
pnpm build
```

The Web event contract includes plan/revision/attempt identity, `step_result`,
`plan_decision`, and `plan_revised`. The reducer retains records, decisions,
and revisions in deduplicated structured projections; there is no compatibility
plan-step dual-fire in the current runtime stream.

Browser-level checks are available separately:

```bash
pnpm test:e2e
```

`shell.spec.ts`, `continuity.spec.ts`, `settings.spec.ts`, `migration.spec.ts`,
and `polish.spec.ts` cover broad product behavior, fault/race injection,
recovery, and visual states with browser-boundary mocks. The gated
`real-api.spec.ts` used by `local-full` exercises the default `/` product shell
against the live Rust API and retains one bounded `/dev/workbench` smoke. The C3
run passed migration, exact A/B continuation with refresh and product
interactions, and the bounded advanced case. The merged CDH live path also
passes wait-for-input Steer, Follow-up enqueue/revoke, final control status, and
completed-session Fork/child continuation (five real-API scenarios total). The
provider runner now uses
the product shell and verifies exact browser-returned job/run IDs, but its
external-provider Web gate was not run for C3.

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
cargo run -p rove-bench -- --suite benchmarks/agent-smoke.json --output-dir .rove/bench
```

The milestone proof map is maintained in
`docs/runtime/acceptance-matrix.md`, which ties M0-M6 criteria to concrete
verification commands.
