# Architecture

## Overview

```
Tauri Desktop ─┐
Next.js Web ───┴─ HTTP / SSE ─→ rove-api ──┐
CLI / TUI / REPL ──────────────────────────┤
rove-bench ────────────────────────────────┤
                                           ▼
                rove-app-bootstrap   config, Project Trust, Engine assembly
                                           ▼
                rove-runtime         Engine, planning, state, tools/MCP, memory, context
                                           ▼
                rove-core            Agent kernel, Tool contract and registry
                                           ▼
                rove-models          model protocols, provider adapters, routing, Fake model
```

Every product surface is a shell: it resolves the workspace, loads the config snapshot, assembles tools and providers, then consumes events from the same Engine. `rove-core::Agent` is only embedded by libraries and tests; the default product entry is `rove-runtime::Engine`, assembled via `rove_app_bootstrap::build_engine`.

## Modules

| Module | Location | Responsibility |
| --- | --- | --- |
| rove-models | `models/` | Normalized `Message`/`ModelToolSchema`/`Usage`/`ModelError`, `ModelClient`/`ModelEvent`, per-provider protocols, routing, health, Fake model. No local dependencies |
| rove-core | `core/` | Runtime-agnostic multi-turn agent kernel: in-memory `Agent`, `AgentEvent`, action parsing, `Tool`/`ToolRegistry`/`ToolDescriptor`, cancel and steer control, the `tool_result.rs` tool-result envelope |
| rove-protocol | `protocol/` | Lowest-level shared types, re-exported by upper layers |
| rove-runtime | `runtime/` | IDs and `RunRequest`/`TaskState`, workspace and path boundaries, `StreamEvent`, state storage (trace/task/report files + SQLite index, repair, cleanup, recovery), context and compaction, memory, built-in tools, MCP proxy, `Executor` and hooks, planning and StepRunner, `Engine`, AgentDefinition and `AGENTS.md` instruction discovery, read-only Review, unified conversation message domain |
| rove-app-bootstrap | `apps/bootstrap/` | `AppConfig` layered loading, user provider catalog `~/.rove/config.toml`, Project Trust store, user state directory and legacy `.rove/` migration, provider factories, product tool registry, Engine assembly |
| rove-cli | `apps/cli/` | The `rove` binary: full-screen TUI by default, plus `repl`, `exec`, `review`, `sessions`, `state`, `trust`, `provider` subcommands |
| rove-product-store | `apps/product-store/` | Product control-plane contract types (`/product/*` request/response, `ProductStore` trait, error codes) and the SQLite implementation (`product.sqlite` schema migrations and repositories), attachment path rules, pricing tables. No HTTP dependency |
| rove-api | `apps/api/` | Axum routes, job lifecycle and SSE, OpenAPI, auth/CORS/rate limiting, product routes and transcript projection, benchmark routes. With `--web-dist`/`ROVE_WEB_DIST` it also serves the built Web bundle on the same origin (API additionally mounted under `/api`, SPA fallback for browser navigation, `/health` for liveness). Re-exports rove-product-store via `rove_api::product` |
| rove-bench | `apps/bench/` | Runs deterministic, network-free benchmarks from JSON definitions |
| rove-desktop | `apps/desktop/` | Tauri 2 host: starts an embedded API on a random loopback port, injects the bearer token before page scripts run, loads the same Web static build, native folder picker, credentials written to the Windows Credential Manager |
| Web | `apps/web/` | Next.js product UI: workspace → session → chat, Inspector, Settings; the server-side `/api/*` proxy injects `ROVE_API_TOKEN` upstream |

## Frontend/backend boundary

- Web talks to `rove-api` over REST + SSE. The contract is the utoipa-generated `/api/openapi.json` at runtime; general conventions in docs/api.md.
- Web's API types are currently hand-written (`apps/web/product/product-api-types.ts`, `apps/web/lib/rove-types.ts`), not generated from OpenAPI. Frontend and backend types must change together when the API changes.
- The browser never retains raw provider keys: keys live in server-side or Desktop env vars and the OS credential store. The single exception is `POST /product/provider-onboarding`, a loopback-bind-only route whose body carries a pasted key transiently into the keyring — the Web form offers it only on a loopback-served page and never stores the value.
- Desktop depends on `rove-api` only within the local package; it does not own a second Engine or ProductStore.

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
5. `Engine::run` produces `StreamEvent`s. The event chain is `ModelEvent → AgentEvent → StreamEvent`; conversion happens in `runtime/src/engine/model_turn.rs`, and only `StreamEvent` is persisted or exposed externally.

The execution shape is "planning on the outside, ReAct on the inside":

- Without planning, `run_unplanned_loop` (`runtime/src/engine/run_loop.rs`) hands the model/tool loop to `rove_core::run_agent_kernel`.
- With planning, `run_planned_loop` (`runtime/src/engine/plan_loop.rs`) runs a StepRunner for each step: a step may contain multiple model/tool rounds and only completes when the model returns `Final`. Every terminal outcome writes an append-only `step_result`, followed by a rule-based `plan_decision`; replacing the plan produces a new immutable revision and emits `plan_revised`. A separate Finalizer then draws the conclusion from the evidence.
- Budgets are managed centrally by `ExecutionPolicy` (`[runtime.execution]`). The retry budget for model calls lives in `runtime/src/engine/recovery.rs`.

One product turn (Web/Desktop): `POST /jobs` carries `product_session_id`; the API resolves the server-held workspace and the exact previous runtime identity, preempts the single active turn, and starts the job through the API supervisor. Messages sent during a run are persisted before delivery: they enter a FIFO and are promoted or withdrawn at safe points; when idle, the earliest pending message is claimed atomically and starts a successor turn. Transcripts read the canonical event projection from each workspace's StateStore.

Recovery: on API startup, jobs in `init`/`running` are marked `interrupted`, and approval and input channels are not rebuilt. An explicit resume creates a new run; an in-flight step with no terminal record gets an `interrupted` record and errors out — it is never re-run.

## Core entities

- Runtime (one `state.sqlite` per workspace): session, job, run, event, report, task_state, plus pending approval/input rows. Schema is authoritative in `runtime/src/state/index.rs` migrations.
- ProductStore (one global `product.sqlite`): workspaces, product sessions, preferences, provider selection, exact product-session→runtime bindings, messages and control, fork lineage, Review, attachment metadata, the `product_events` journal. Schema is authoritative in `apps/product-store/src/store/schema.rs` migrations. ProductStore holds product control state and does not duplicate canonical events.

## State directories

```
<data_root>/                          Windows %LOCALAPPDATA%\rove, macOS ~/Library/Application Support/rove,
  product.sqlite                      Linux $XDG_DATA_HOME/rove; ROVE_DATA_ROOT overrides (must be absolute)
  attachments/<product_session_id>/<attachment_id>
  workspaces/<storage_key>/           storage_key = first 16 chars of a stable hash of canonical root path + kind (repo/folder)
    workspace.json  state.sqlite  mcp_servers.json  circuit_breakers.json  repl_history
    runs/<run_id>/{trace.jsonl, task_state.json, report.json, tool_artifacts/}
    memory/{MEMORY.md, topics/, sessions/}
    session-model-selections/  tasks/<name>/  .migration/
~/.rove/config.toml                   user provider catalog (ROVE_CONFIG_ROOT override); stores credential references only
```

- Project directories only carry Trust-gated project config: `.rove/config.toml`, `.env`, `AGENTS.md`, AgentDefinition packages.
- Moving or renaming a workspace produces a new key and is treated as a new workspace.
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
