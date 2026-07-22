# Runtime Architecture

`rove` is a local-first runtime with remote-ready seams. The default mode is local: CLI runs in the current workspace, API binds to `127.0.0.1:8787`, and state is written under `.rove/`.

The repository manifest is currently a transitional Cargo Workspace containing
the existing root `rove` compatibility package and the independent
`rove-models`, `rove-core`, `rove-runtime`, and `rove-app-bootstrap` packages; the root package
remains the default member. Shared package metadata and dependency versions are
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
optional RAG, first-party AppConfig, and app packages still remain at the
transitional root paths documented below.

## Shape

```text
CLI / TUI / API / Web
    -> root Engine compatibility re-export
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
```

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
   approvals, inputs, planner state, and cancellation are processed. Planned
   attempts carry stable plan/revision/attempt identity and end with canonical
   `step_result` and `plan_decision` events before compatibility plan-step
   completion/failure events. Replacement work emits a linked
   `plan_revised` event rather than another initial-plan event.
6. `TraceWriter` writes append-only trace events. `RunArtifactRecorder`
   materializes step records, plan decisions, immutable revisions, and the
   active attempt into task state, stores bounded lifecycle metadata in the
   prompt checkpoint, and includes the lifecycle projections in the report.
7. The API adds a live job registry for active handles and reads SQLite for persisted job state and SSE replay after restart.

## Boundary Rules

- Core code emits normalized runtime events and does not depend on the CLI, API, or web UI.
- Provider adapters normalize provider-specific streams into `ModelEvent`.
- `rove-models` owns the normalized message/tool/usage/error protocol, provider
  adapters, routing, health, and Fake Model without depending on another local
  project package. The root facade re-exports these contracts while
  AppConfig-driven construction remains transitional product assembly.
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
- Model-visible `rove_models::ToolSchema` is separate from operational
  `rove_core::ToolDescriptor`; provider payloads receive only the model schema.
- Local built-in tool implementations, runtime-specific Workspace, Memory,
  policy and input invocation services, MCP proxy, `Executor`, hooks, tool
  turns, planning, and Engine live in `rove-runtime`. Product registry assembly
  and optional RAG remain transitional root services.
- The event chain is `ModelEvent -> AgentEvent -> StreamEvent`.
  `rove-runtime` owns the canonical `StreamEvent` type and performs the
  synchronous translation in `runtime/src/model_turn.rs`. Only `StreamEvent` is
  persisted or exposed by apps.
- Files remain the readable source artifacts; SQLite is the query/replay index.
- A `step_result` trace event is the append-only terminal fact. The task-state
  ledger and report records are projections and must not overwrite prior
  attempts during replanning.
- Every newly handled terminal `step_result` has exactly one correlated
  rule-first `plan_decision`; replacement work is represented by an immutable
  parent-linked `plan_revised` event. These are canonical stream events shared
  by persistence, API/SSE, terminal views, and Web.
- RAG is feature-gated behind `--features rag`; default builds keep stub schemas and clear disabled-feature errors.

## State Artifacts

```text
.rove/
  state.sqlite
  runs/<run_id>/trace.jsonl
  runs/<run_id>/task_state.json  # plan cursor + lifecycle projections
  runs/<run_id>/report.json      # aggregate + records/decisions/revisions
  memory/MEMORY.md
  memory/topics/*.md
  memory/sessions/<session_id>.md
  rag_manifest.json
  rag_index_log.jsonl
  rag_eval/<run_id>.json
```

The RAG files are only produced when the `rag` feature is enabled and indexing/eval commands are run.

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
