# Architecture

For a self-contained 10-minute description aimed at a reviewer, start with [docs/system-overview.md](system-overview.md); this file is the detailed reference. A rendered layer diagram lives at [docs/architecture.svg](architecture.svg).

## Overview

```
Tauri Desktop ─┐
Next.js Web ───┴─ HTTP / SSE ─→ rove-api ──┐
CLI / TUI / REPL ──────────────────────────┤
rove-bench ────────────────────────────────┤
                                           ▼
                rove-app-bootstrap   config, Project Trust, Engine assembly
                                           ▼
                rove-engine          Engine, planning, delegate turn-driver, context
                                           ▼
                rove-tools           built-in tools, Executor pipeline, hooks, MCP client
                                           ▼
                rove-state           durable run state, trace, SQLite index, durable memory
                rove-review          read-only Review contracts, target snapshots, submission tools
                                           ▼
                rove-agents          AgentDefinition, run profiles, AGENTS.md discovery, procedure catalog
                                           ▼
                rove-foundation      serialized identity/lifecycle data, workspace boundary types
                                           ▼
                rove-core            Agent kernel, Tool contract and registry
                                           ▼
                rove-models          model protocols, provider adapters, routing, Fake model
```

Every product surface is a shell: it resolves the workspace, loads the config snapshot, assembles tools and providers, then consumes events from the same Engine. `rove-core::Agent` is only embedded by libraries and tests; the default product entry is `rove_engine::Engine`, assembled via `rove_app_bootstrap::build_engine`.

TUI saved history reads bounded canonical event snapshots from StateIndex and reuses the terminal projection. Resuming restores the selected run; an independent read-only browser moves between session-scoped run neighbors without changing execution state. Schema migration 6 adds the `(session_id, started_at, run_id)` index for these reads. Partial-history notices remain attached to restored transcript segments across subsequent turns; historical approvals never recreate interaction channels.

## Modules

| Module | Location | Responsibility |
| --- | --- | --- |
| rove-models | `crates/models/` | Normalized `Message`/`ModelToolSchema`/`Usage`/`ModelError`, `ModelClient`/`ModelEvent`, per-provider protocols, routing, health, Fake model. No local dependencies |
| rove-core | `crates/core/` | Runtime-agnostic multi-turn agent kernel: in-memory `Agent`, `AgentEvent`, action parsing, `Tool`/`ToolRegistry`/`ToolDescriptor`, cancel and steer control, the `tool_result.rs` tool-result envelope |
| rove-protocol | `crates/protocol/` | Lowest-level shared types, re-exported by upper layers |
| rove-foundation | `crates/foundation/` | Serialized identity/lifecycle data extracted from runtime: `RuntimeIdentity`/`RunModelSnapshot`, `StreamEvent`, `Session`, `types` (IDs, plans, providers), `secrets`, `capability`, workspace `boundary`/`root`, `prompt_metadata`, `hashing`, agent leaf identities (`AgentProfileIdentity`/`AgentDiagnostic`/`AgentSelector`/`ProcedureReference`/`RiskLevel`/`SideEffect`/`ProcedureTrust`), MCP snapshot records, environment records, planning/execution lifecycle records, registered user-input plumbing. Depends only on rove-core/models/protocol; no behaviour. `TaskState`/`PromptCheckpoint`/`RunRequest` live in rove-state (they embed `AgentRuntimeProfile`) |
| rove-state | `crates/state/` | Durable run state extracted from runtime: `state/` (StateStore, trace/`trace_reader`, `task_state` snapshots, `report`, SQLite `index`, `migration_lock`, repair/`reconcile`, `resume`, `initial_history`, `reverse_trace_scanner`, Tool Artifacts), `memory/` (durable/session/layered memory and paths), and the `TaskState`/`RunRequest`/`PromptCheckpoint`/`MessageDeliveryRecord` snapshot types plus the `inherits_session_breaker` session-breaker predicate. Depends on rove-foundation/agents/core/models/protocol; never on rove-engine |
| rove-review | `crates/review/` | Read-only Review domain extracted from runtime: `target` (ReviewTargetSpec/Entry/Snapshot/Summary, git capture plumbing, `resolve_external_state_root`, `descriptor_allowed`, `is_review_mode`), `finding` (ReviewFinding/input contracts, `sanitize_findings`), `result` (ReviewResult/Conclusion/Stats/Unchecked/RuntimeEvidence, `finalize_result*`, `apply_runtime_outcome`), `tool` (`ReviewSubmission`/`ReviewSubmissionStore` plus the snapshot-backed read/submit `Tool` impls). Depends on rove-foundation/core/models/protocol; never on rove-engine |
| rove-tools | `crates/tools/` | Built-in tools extracted from runtime: coding/fs/search/shell/repository/memory/history/request_input tools, the `Executor` pipeline, `hooks` (pre/post-tool and post-run incl. `SessionMemoryHook`), `runtime_context` (the `RuntimeToolServices` context extension), the MCP client (`mcp/` + `mcp_proxy`/`mcp_config`), and the delegation data half (`delegate`: `SubRunRequest`/`Outcome`/`Error`, `SubagentRegistry`, `SubUsageLedger`, `SubRunApprovalParts`, the `SubRunSpawn` trait, and `DelegateTool` itself). The delegate turn-driver (`SubRunSpawner`/`SubagentKernelHost`) stays in rove-engine's `engine::subrun` because it must call the engine's turn primitives; `DelegateTool` reaches it through the `SubRunSpawn` trait-object `ToolContext` extension, keeping the edge one-directional. Depends on rove-foundation/agents/state/review/core/models/protocol/tools-text; never on rove-engine |
| rove-engine | `crates/engine/` | Context and compaction, planning and StepRunner, `Engine`, unified conversation message domain, and the delegate turn-driver (`engine::subrun`: `SubRunSpawner`/`SubagentKernelHost`, implementing `rove_tools::delegate::SubRunSpawn`). Renamed from `rove-runtime`; the compat re-export shims are gone, so downstream imports the extracted crates directly |
| rove-agents | `crates/agents/` | `AgentDefinition` manifest parsing/validation, `AgentRuntimeProfile` (the compiled run snapshot with `allows_capability`/`prompt_slot`/`identity()`), `AgentPackage` bounded loading, `AGENTS.md` instruction discovery (`InstructionBundle`/overlays), the typed procedure document/catalog/selection/hydration pipeline, `AgentActivation` prompt assembly. Re-exports foundation's agent leaf identities (`AgentProfileIdentity`/`AgentDiagnostic`/`AgentSelector`/`ProcedureReference`/`RiskLevel`/`SideEffect`/`ProcedureTrust`) and `hashing`/`selector` at their historic `rove_agents::*` paths. Depends only on rove-foundation/core/models/protocol; never on rove-engine |
| rove-app-bootstrap | `apps/bootstrap/` | `AppConfig` layered loading, user provider catalog `~/.rove/config.toml`, Project Trust store, user state directory and legacy `.rove/` migration, provider factories, product tool registry, Engine assembly |
| rove-cli | `apps/cli/` | The `rove` binary: full-screen TUI by default, plus `repl`, `exec`, `review`, `sessions`, `state`, `trust`, `provider` subcommands |
| rove-product-store | `apps/product-store/` | Product control-plane contract types (`/product/*` request/response, `ProductStore` trait, error codes) and the SQLite implementation (`product.sqlite` schema migrations and repositories), attachment path rules, pricing tables. No HTTP dependency |
| rove-api | `apps/api/` | Axum routes, job lifecycle and SSE, OpenAPI, auth/CORS/rate limiting, product routes and transcript projection, benchmark routes. With `--web-dist`/`ROVE_WEB_DIST` it also serves the built Web bundle on the same origin (API additionally mounted under `/api`, SPA fallback for browser navigation, `/health` for liveness). Re-exports rove-product-store via `rove_api::product` |
| rove-bench | `apps/bench/` | Runs deterministic, network-free benchmarks from JSON definitions |
| rove-desktop | `apps/desktop/` | Tauri 2 host: starts an embedded API on a random loopback port, injects the bearer token before page scripts run, loads the same Web static build, native folder picker, Windows native credential entry; macOS/Linux reuse the shared loopback form; credentials stored through the shared OS keyring service |
| Web | `apps/web/` | Next.js product UI: workspace → session → chat, Inspector, Settings; the server-side `/api/*` proxy injects `ROVE_API_TOKEN` upstream |

## Frontend/backend boundary

- Web talks to `rove-api` over REST + SSE. The contract is the utoipa-generated `/api/openapi.json` at runtime; general conventions in docs/api.md.
- Web API types are generated from the OpenAPI snapshot into `apps/web/generated/api-types.ts`; `product-api-types.ts` and `rove-types.ts` expose aliases and boundary guards. API changes update Rust types and annotations first, then the snapshot and generated frontend types.
- The browser never retains raw provider keys: keys live in server-side or Desktop env vars and the OS credential store. The single exception is `POST /product/provider-onboarding`, a loopback-bind-only route whose body carries a pasted key transiently into the keyring — the Web form offers it only on a loopback-served page and never stores the value.
- Desktop depends on `rove-api` only within the local package; it does not own a second Engine or ProductStore.
- API startup ownership is held by the ProductStore directory lock and is propagated through runtime index/store workers. A cancelled async SQLite waiter leaves the ownership guard with its blocking database worker until that worker finishes. Embedders must drain before destroying their runtime; arbitrary runtime abortion is not a supported handoff between hosts.
- Desktop window-close requests enter Tauri's application-exit gate. It starts the embedded API cancellation/drain once and prevents further exit requests while pending. The final exit callback joins the same drain before Tauri cleanup, covering macOS native Quit which bypasses the request event. Completion or the existing five-second abort deadline permits process exit. This is host lifecycle coordination, not a second task-state contract; forced process termination is not intercepted.

## Authentication and permissions

- The API binds `127.0.0.1:8787` by default. Binding a non-loopback address requires a bearer token unless `ROVE_API_UNSAFE_REMOTE_WITHOUT_AUTH` is explicitly set.
- With `api.token_auth` configured, business routes require `Authorization: Bearer <token>`. The token is registered in the in-process secret manager and never appears in SSE frames or error bodies.
- Project Trust is restricted by default and grants per exact root path and capability: project `.env`, `.rove/config.toml`, MCP processes, hooks, AgentDefinition packages, and external paths only take effect after the workspace and its capability summary are trusted. The trust store lives at `%LOCALAPPDATA%\rove\project-trust.sqlite` (overridable via `ROVE_PROJECT_TRUST_STORE`).
- Tool approval policy: `ask` / `auto` / `never`. Destructive tools always go through approval; a denied approval fails closed.

## Core data flow

One run:

1. `Workspace::detect` resolves the workspace root; the state directory comes from the user state contract.
2. `AppConfig::load` resolves Project Trust first, then merges config in the order "defaults → user directory → trusted project config → environment variables → CLI/API overrides".
3. The shell builds `ModelClient`, a validated `ToolRegistry`, `ContextManager`, and `StateStore`. Each tool's schema is pinned at registration, and the Engine derives an immutable capability snapshot from the registry.
4. `StateStore::start_run` creates the run directory and writes session/job/run identities into SQLite.
5. `Engine::run` produces `StreamEvent`s. The event chain is `ModelEvent → AgentEvent → StreamEvent`; conversion happens in `crates/engine/src/engine/model_turn.rs`, and only `StreamEvent` is persisted or exposed externally.

The execution shape is "planning on the outside, ReAct on the inside":

- Without planning, `run_unplanned_loop` (`crates/engine/src/engine/run_loop.rs`) hands the model/tool loop to `rove_core::run_agent_kernel`.
- With planning, `run_planned_loop` (`crates/engine/src/engine/plan_loop.rs`) runs a StepRunner for each step: a step may contain multiple model/tool rounds and only completes when the model returns `Final`. Every terminal outcome writes an append-only `step_result`, followed by a rule-based `plan_decision`; replacing the plan produces a new immutable revision and emits `plan_revised`. A separate Finalizer then draws the conclusion from the evidence.
- Budgets are managed centrally by `ExecutionPolicy` (`[runtime.execution]`). The retry budget for model calls lives in `crates/engine/src/engine/recovery.rs`.

One product turn (Web/Desktop): `POST /jobs` carries `product_session_id`; the API resolves the server-held workspace and the exact previous runtime identity, preempts the single active turn, and starts the job through the API supervisor. Messages sent during a run are persisted before delivery: they enter a FIFO and are promoted or withdrawn at safe points; when idle, the earliest pending message is claimed atomically and starts a successor turn. Transcripts read the canonical event projection from each workspace's StateStore.

Recovery: API construction first takes exclusive OS file ownership of the ProductStore directory. A second host fails before recovery, even when it uses another port or workspace. Jobs in `init`/`running` are then marked `interrupted`, and approval and input channels are not rebuilt. An explicit resume creates a new run; an in-flight step with no terminal record gets an `interrupted` record and errors out — it is never re-run. Every API state/router clone and tracked maintenance writer retains ownership; embedders call `ApiState::shutdown()` and drop their clones before opening a new host. The OS releases ownership when the process dies; the empty lock file is intentionally retained.

## Core entities

- Runtime (one `state.sqlite` per workspace): session, job, run, event, report, task_state, plus pending approval/input rows. Schema is authoritative in `crates/state/src/state/index.rs` migrations.
- ProductStore (one global `product.sqlite`): workspaces, product sessions, preferences, provider selection, exact product-session→runtime bindings, messages and control, fork lineage, Review, attachment metadata, the `product_events` journal. Schema is authoritative in `apps/product-store/src/store/schema.rs` migrations. ProductStore holds product control state and does not duplicate canonical events.

## State directories

```
<data_root>/                          Windows %LOCALAPPDATA%\rove, macOS ~/Library/Application Support/rove,
  .rove-api.lock                     OS-held API ownership; existence alone never means busy
  product.sqlite                      Linux $XDG_DATA_HOME/rove; ROVE_DATA_ROOT overrides (must be absolute)
  attachments/<product_session_id>/<attachment_id>
  workspaces/<storage_key>/           storage_key = first 16 chars of a stable hash of canonical root path + kind (repo/folder)
    workspace.json  state.sqlite  mcp_servers.json  circuit_breakers.json  repl_history
    runs/<run_id>/{trace.jsonl, task_state.json, report.json, tool_artifacts/}
    memory/{MEMORY.md, topics/, sessions/}
    session-model-selections/  tasks/<name>/  .migration/
<platform Desktop config/state/log roots>  ROVE_DESKTOP_ROOT optionally groups these under config/, state/, logs/
~/.rove/config.toml                   user provider catalog (ROVE_CONFIG_ROOT override); stores credential references only
```

- Project directories only carry Trust-gated project config: `.rove/config.toml`, `.env`, `AGENTS.md`, AgentDefinition packages.
- Moving or renaming a workspace produces a new key and is treated as a new workspace.
- API ownership coordinates cooperating API hosts, including Desktop. It does not serialize external editors, CLI task execution, or custom runtime paths deliberately shared across different data roots.
- `rove state paths` shows the resolved layout; `rove state migrate [--apply] [--on-conflict backup-target] [--prune-legacy]` migrates a legacy `.rove/`. Dry-run by default with zero writes; idempotency is judged by per-file sha256; SQLite is snapshotted via `VACUUM INTO` with paths rewritten; conflicts are never silently overwritten.

## Memory and context

- Three layers: working memory (in-process messages), session memory (`memory/sessions/<session_id>.md`), and persistent memory (`MEMORY.md` + `topics/*.md`, managed by `save_memory`/`read_memory`/`reindex_memory`, which reject suspected secrets and ephemeral content).
- Recall is CJK-aware lexical scoring (smoothed IDF + field weights + confidence + recency boost), capped by `memory.recall_limit`. There is no vector RAG; workspace retrieval goes through tools.
- Prompt order: system → persistent memory → session memory → compaction summary → recent history → current user message. A native tool call and its result are selected as one unit.
- Compaction: triggers when over budget; can be model-generated (`rove.compaction.v3`, seven fields), falls back to a deterministic summary marked as degraded on failure, and consecutive failures trip a circuit breaker. Worth-persisting content is flushed to session memory before compaction.
- `ModelClient` is a stateless request boundary; the stable prefix must be sent in full on every request.

## Deployment

- Local single user only: no hosted service, accounts, or billing.
- Desktop: Windows MSI/NSIS builds and the release pipeline work; signing, the installed end-to-end flow, and macOS/Linux packaging are not yet verified.
- CLI: `cargo install --path apps/cli` installs `rove`.
- Config sources: defaults → `~/.rove/config.toml` → trusted project `.rove/config.toml` → `ROVE_*` environment variables → CLI/API overrides.

## Verified file rollback

Built-in file writes and edits add optional SHA-256 `before_version` / `after_version` fields to canonical `ToolMutation` records. Old events and snapshots decode with absent fields and serialize unchanged. Diff rendering retains CRLF and final-newline boundaries. Product rollback holds a cancellation-safe session maintenance claim; it checks exact current bytes, reconstructs a bounded single-file reverse patch in memory, verifies original bytes, and publishes only the requested ordinary file. Display diffs can be truncated or redacted, so they alone cannot authorize restoration. Legacy incomplete evidence stays viewable but cannot drive rollback. External concurrent writers remain outside the session claim.
