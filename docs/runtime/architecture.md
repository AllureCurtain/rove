# Runtime Architecture

`rove` is a local-first runtime with remote-ready seams. The default mode is local: CLI runs in the current workspace, API binds to `127.0.0.1:8787`, and state is written under `.rove/`.

The repository manifest is a modular Cargo Workspace containing
a virtual Cargo Workspace of `rove-models`, `rove-core`, `rove-runtime`,
`rove-app-bootstrap`, `rove-cli`, `rove-api`, `rove-bench`, and
`rove-integration-tests`; the default member is `apps/cli`. Shared package metadata and dependency versions are
defined at Workspace scope. `rove-core` is the in-memory embedding layer and
depends only on `rove-models`. The first verified `rove-runtime` slice depends
only on those two packages and owns runtime identity, task/execution contracts,
Workspace/path safety, prompt metadata, and approval/input provider contracts.
The same crate now also owns canonical `StreamEvent` and StateStore, trace,
task/report artifacts, SQLite indexing, repair, cleanup, resume,
context/compaction, session/durable memory services, local built-in tools,
their invocation adapters, the existing MCP proxy, the tool Executor pipeline,
pre/post-tool plus post-run hooks, planning/step coordination, durable event
translation, and the persistent Engine facade. Product tool-registry assembly,
first-party AppConfig and product assembly live in `apps/bootstrap`; product apps live under `apps/`.

## Shape

```text
CLI / TUI / API / Web
    -> rove-app-bootstrap::build_engine / tool_registry
        -> rove-runtime Engine / planning / tool turns / hooks / Executor
        -> rove-runtime context / memory / identity / execution / state / events
        -> rove-core model turn / ToolRegistry contracts
            -> rove-models ModelClient / RoutingModelClient
        -> runtime approval and input adapters
        -> Memory loaders
        -> StateStore

External embedding
    -> rove-core::Agent
        -> rove-models::ModelClient
        -> custom ToolRegistry / ToolPolicy
        -> in-memory AgentEvent

StateStore
    -> .rove/runs/<run_id>/*
    -> .rove/state.sqlite

API product control plane
    -> <api state_dir>/product.sqlite
    -> product workspace/session/profile/preferences/control catalog
    -> exact product-session -> runtime session/job/run bindings
    -> bounded live steer delivery + durable follow-up drain
    -> transcript read projection over per-workspace StateStore events
```

Product shells assemble and run `rove-runtime::Engine`. `rove-core::Agent` is
embed-only for libraries and tests; it is not the product default entry.

| Layer | Default consumer | Role |
|---|---|---|
| `rove-models` | everyone below | Wire protocols, `ModelClient`, routing |
| `rove-core` | libraries / tests | Embeddable `Agent`, tool contracts |
| `rove-runtime` | CLI / API / Web / Bench | Durable `Engine`, plan, state, memory, tool impls |
| `apps/*` | end users | Product shells |

The interface layers are shells. They detect the workspace, load a config snapshot, construct tools and providers, then consume the same engine events.

The optional full-screen TUI adds only a presentation shell around that path:
`EventStream -> TuiAction -> reducer -> shared Engine`, followed by an awaited
bounded run-update sink into Ratatui. Approval and input providers additionally
register process-local responders through a bounded channel; the TUI exposes a
modal only when that request matches the canonical event by kind and ID. It does
not introduce a second event lifecycle or persistence format. Modal responses
are armed only after a visible-frame and held-key boundary; terminals that
cannot supply trustworthy interaction events fail closed.

## Core Flow

1. `Workspace::detect` chooses the workspace root and `.rove` state directory.
2. `AppConfig::load` merges defaults, `.rove/config.toml`, environment variables, and explicit CLI/API overrides.
3. The interface builds a `ModelClient`, `ToolRegistry`, `ContextManager`, and `StateStore`.
4. `StateStore::start_run` creates a run directory and indexes session/job/run identity in SQLite.
5. `Engine::run` emits `StreamEvent` values while model events, tool calls,
   approvals, inputs, planner state, cancellation, and bounded in-flight steer
   delivery are processed. A steer crosses only a loop safe point before the
   next model turn; it never lands inside a tool side effect. Planned
   attempts carry stable plan/revision/attempt identity and end with canonical
   `step_result` and `plan_decision` events. Replacement work emits a linked
   `plan_revised` event rather than another initial-plan event. Compatibility
   dual-fire `plan_step_completed` / `plan_step_failed` events are not emitted.
6. `TraceWriter` writes append-only trace events. `RunArtifactRecorder`
   materializes step records, plan decisions, immutable revisions, and the
   active attempt into task state, stores bounded lifecycle metadata in the
   prompt checkpoint, and includes the lifecycle projections in the report.
7. The API adds a live job registry for active handles and reads SQLite for persisted job state and SSE replay after restart.
8. The API also opens one application-global ProductStore at
   `<configured state_dir>/product.sqlite`. Product routes own catalog,
   preferences, migration receipts, active-turn claims, exact runtime
   bindings, and idempotent product-session controls there. Live steers are
   persisted before their bounded runtime delivery. Follow-ups are durable
   queue records: an atomic terminal-turn transition claims the oldest pending
   follow-up and starts its successor through the API supervisor. Canonical
   events and run artifacts remain in each selected execution workspace.

CDH G2 extends that catalog with immutable fork provenance. A fork is accepted
only after the API verifies the selected parent run's durable terminal boundary;
the child records the parent product session and exact source runtime
session/job/run plus terminal sequence. Its inherited prefix is a read-only
projection of source canonical events, never a copied child event ledger. The
first child turn seeds its prompt/history from the verified source TaskState but
starts with fresh runtime identities and no parent `resumed_from_run_id` lineage.
Fork retries are idempotent, and the provenance/replay record survives parent
product-session deletion.

For a C0 product turn, `POST /jobs.product_session_id` resolves the server-owned
workspace and exact prior runtime identity, claims one active turn, and launches
the job through an API-owned start task and supervisor. Shutdown closes and
drains start tasks before supervisors and job handles. A transcript request
walks the product session's immutable ordered bindings and reads canonical
events from the corresponding workspace StateStores; unavailable or
inconsistent facts are reported as typed partial reasons.

The M1 browser migration has two boundaries. Preparation is limited to 30
seconds and validates/canonicalizes eligible referenced workspaces and runtime
facts.
Once accepted, apply runs under an API-owned supervisor and is not cancelled by
an HTTP disconnect. ProductStore persists the first preference revision
baseline for the idempotency key and consumes it atomically with the receipt;
runtime stores are canonicalized, sorted, and reserved before verified bindings
are committed.

## Boundary Rules

- Core code emits normalized runtime events and does not depend on the CLI, API, or web UI.
- Provider adapters normalize provider-specific streams into `ModelEvent`.
- `rove-models` owns the normalized message/tool/usage/error protocol, provider
  adapters, routing, health, and Fake Model without depending on another local
  project package. Product assembly builds clients through
  `apps/bootstrap` (`AppConfig` + named profiles only).
- `rove-core` owns the in-memory `Agent`, `AgentEvent`, action/parser and model
  turn, cancellation/control, `Tool`/`ToolRegistry`, `ToolDescriptor`, and
  runtime-neutral policy hook. It depends only on `rove-models` and creates no
  workspace or state directory.
- `rove-runtime` owns `SessionId`/`JobId`/`RunId`, `RunRequest`, `TaskState`,
  prompt checkpoints, execution-policy and plan-ledger data, Workspace/path
  enforcement, prompt metadata/runtime identity, approval/input provider
  contracts, the task-local input registration context, canonical
  `StreamEvent`, context/compaction, session/durable memory, local built-in
  filesystem/shell/memory/input tools, invocation adapters, the existing
  stdio/legacy-SSE MCP proxy, the tool `Executor` pipeline, pre/post-tool and
  post-run hooks, planning/step coordination, durable event translation, the
  persistent `Engine` facade, and state/trace/artifact/SQLite/repair/resume
  services. Its only local dependencies are `rove-models` and `rove-core`.
- Model-visible `rove_models::ModelToolSchema` is separate from operational
  `rove_core::ToolDescriptor`; provider payloads receive only the model schema.
- Local built-in tool implementations, runtime-specific Workspace, Memory,
  policy and input invocation services, MCP proxy, `Executor`, hooks, tool
  turns, planning, and Engine live in `rove-runtime`. Product registry assembly
  live in `apps/bootstrap` and first-party apps.
- The event chain is `ModelEvent -> AgentEvent -> StreamEvent`.
  `rove-runtime` owns the canonical `StreamEvent` type and performs the
  synchronous translation in `runtime/src/engine/model_turn.rs`. Only `StreamEvent` is
  persisted or exposed by apps.
- Files remain the readable source artifacts; SQLite is the query/replay index.
- ProductStore is API-global product-control state, not another event store. It
  may retain product IDs, safe settings, mappings, claims, controls, migration
  preparations, and receipts, but it does not copy canonical trace/task/report
  truth. API-originated control lifecycle facts are appended through the same
  canonical stream contract rather than a product-private event lifecycle.
- Verified migration bindings use canonical workspace-contained runtime
  database/artifact paths when external paths are disabled. SQLite guards use
  no-follow opens and reject symlinked parent paths before read or reservation.
- A `step_result` trace event is the append-only terminal fact. The task-state
  ledger and report records are projections and must not overwrite prior
  attempts during replanning.
- Every newly handled terminal `step_result` has exactly one correlated
  rule-first `plan_decision`; replacement work is represented by an immutable
  parent-linked `plan_revised` event. These are canonical stream events shared
  by persistence, API/SSE, terminal views, and Web.
- Built-in vector RAG is not part of the product; workspace context comes from tools and layered file memory.

## State Artifacts

```text
<configured API state_dir>/
  product.sqlite                 # API-global product-control state

<execution workspace>/.rove/
  state.sqlite                   # canonical runtime query/replay index
  runs/<run_id>/trace.jsonl
  runs/<run_id>/task_state.json  # plan cursor + lifecycle projections
  runs/<run_id>/report.json      # aggregate + records/decisions/revisions
  memory/MEMORY.md
  memory/topics/*.md
  memory/sessions/<session_id>.md
```


## Restart Semantics

On API startup, SQLite is initialized and any jobs/runs still marked `init` or
`running` are marked `interrupted`. Historical jobs remain queryable through
`/jobs/{job_id}/state`, and historical SSE events are replayed from SQLite
through `/jobs/{job_id}/events`.

Active handles such as cancellation tokens, task handles, broadcast senders,
approvals, and input channels live only in memory and are not reconstructed
after restart. When an explicit resume finds a persisted planned-step attempt
without a terminal record, it appends an `interrupted` record and stops with an
error instead of repeating model/tool work. A persisted successful terminal
record advances the materialized plan without replaying that step. Resume does
not yet reconcile trace events written after the latest task-state snapshot.
Legacy snapshots with a mutable plan but no lifecycle chain are wrapped once as
an immutable revision-zero plan before new transitions are evaluated.

Product controls have separate conservative recovery rules. On ProductStore
open, a stale follow-up claim without a reserved runtime run is returned to the
durable pending queue; an already reserved run is not replayed and is surfaced
as `needs_attention`. Once the API is ready, only idle product sessions with a
pending follow-up are scheduled for drain. Pending or safe-point-accepted
steers are classified as dropped at a terminal boundary when no next model turn
can apply them.
