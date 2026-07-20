# Runtime Architecture

`rove` is a local-first runtime with remote-ready seams. The default mode is local: CLI runs in the current workspace, API binds to `127.0.0.1:8787`, and state is written under `.rove/`.

## Shape

```text
CLI / TUI / API / Web
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
   `step_result` before compatibility plan-step completion/failure events.
6. `TraceWriter` writes append-only trace events. `RunArtifactRecorder`
   materializes the step ledger and active attempt into task state, stores
   bounded ledger metadata in the prompt checkpoint, and includes terminal
   step records in the report.
7. The API adds a live job registry for active handles and reads SQLite for persisted job state and SSE replay after restart.

## Boundary Rules

- Core code emits normalized runtime events and does not depend on the CLI, API, or web UI.
- Provider adapters normalize provider-specific streams into `ModelEvent`.
- Tool execution happens through `Executor` and `ToolRegistry`; approval policy is passed through `ToolContext`.
- Files remain the readable source artifacts; SQLite is the query/replay index.
- A `step_result` trace event is the append-only terminal fact. The task-state
  ledger and report records are projections and must not overwrite prior
  attempts during replanning.
- RAG is feature-gated behind `--features rag`; default builds keep stub schemas and clear disabled-feature errors.

## State Artifacts

```text
.rove/
  state.sqlite
  runs/<run_id>/trace.jsonl
  runs/<run_id>/task_state.json  # plan cursor + materialized step ledger
  runs/<run_id>/report.json      # aggregate + terminal step records
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
