# Implementation Comparison - 2026-05-23

This compares the current `main` implementation with the original design in:

- `docs/04-架构与路线图.md`
- `docs/05-下一步-统一执行内核.md`
- `docs/06-请求生命周期.md`

## Summary

`main` is already beyond the original M0-M2 kernel plan and includes partial M3, M4, M5, and M6 surfaces. The largest difference is shape: the original docs describe an eventual `EngineDeps` dependency graph and richer provider routing, while the current implementation remains a simpler single-crate engine. `RunStream` and cooperative cancellation now exist, but the full app-level token tree and provider-routing design are still open.

## Implemented Close To The Design

| Area | Original design | Current evidence | Notes |
|---|---|---|---|
| Core as library, interfaces as shells | CLI/API/Web consume the same event stream | `src/core/engine.rs`, `src/interfaces/cli/oneshot.rs`, `src/interfaces/api/mod.rs`, `web-ui/lib/rove-client.ts` | Core does not import `interfaces`; CLI/API consume `StreamEvent`. |
| Stream event model | Tagged enum serialized for CLI/SSE/Web | `src/core/events.rs` | Uses `#[serde(tag = "type", rename_all = "snake_case")]`; includes run, LLM, tool, approval, plan, and completion events. |
| Workspace boundary | Folder/Repo detection and `.rove` state root | `src/core/workspace.rs`, `tests/e2e.rs` | Detects nearest git root and stores state under `<root>/.rove`. |
| Run identity | ULID newtypes for session/job/run/call | `src/core/types.rs` | Current `RunRequest` carries explicit `session_id`, `job_id`, and `run_id`. |
| `RunStream` handle | `Engine::run(req) -> RunStream` with IDs and `cancel()` | `src/core/engine.rs`, `tests/e2e.rs` | `RunStream` exposes `session_id()`, `job_id()`, `run_id()`, `cancel()`, and cancels on drop. |
| Trace/state/report artifacts | `.rove/runs/<run_id>/{trace.jsonl,task_state.json,report.json}` | `src/state/trace.rs`, `src/state/store.rs`, `src/state/artifacts.rs`, `tests/e2e.rs`, `tests/api.rs` | Snapshot writes are schema-versioned; report includes workspace and identity metadata. |
| Tool pipeline | schema -> validate -> hooks -> approval/boundary -> exec -> post-hook | `src/core/executor.rs`, `src/core/boundary.rs`, `src/hooks/mod.rs` | Implemented with pre/post hooks and destructive-tool approval policy. |
| File and shell tools | Built-in workspace tools with boundary checks | `src/tools/fs.rs`, `src/tools/shell.rs`, `tests/e2e.rs` | File tools stay inside workspace; shell rejects empty/NUL commands and destructive calls obey policy. |
| Resume snapshots | `task_state.json` supports resume | `src/state/store.rs`, `src/interfaces/cli/oneshot.rs`, `tests/e2e.rs` | Supports `--resume latest` and `--resume <run_id>`. |
| CLI fast paths | Lightweight subcommands avoid full engine setup | `src/interfaces/cli/config.rs`, `src/interfaces/cli/index.rs`, `src/interfaces/cli/sessions.rs`, `src/main.rs`, `tests/cli_config.rs`, `tests/cli_index.rs`, `tests/cli_sessions.rs` | `dump-config`, `index`, and `sessions` now run before workspace/model/tool setup. |
| Planner | Persisted plan and step events | `src/core/planner.rs`, `src/core/engine.rs`, `prompts/planner.md`, `tests/e2e.rs` | Includes replanning after failed planned steps. |
| M5 API | Axum jobs API with SSE and cancel | `src/interfaces/api/mod.rs`, `src/bin/rove-api.rs`, `tests/api.rs` | Implements `POST /jobs`, `GET /jobs/{id}/events`, `GET /jobs/{id}/state`, and `POST /jobs/{id}/cancel`. |
| API approval flow | Pending destructive-tool approval over HTTP | `src/interfaces/api/mod.rs`, `tests/api.rs` | Adds `POST /jobs/{id}/approvals/{call_id}`, which is beyond the original M5 baseline. |
| Web workbench | Next.js workbench consumes SSE and shows plan/tools/trace | `web-ui/app/page.tsx`, `web-ui/components/rove-workbench.tsx`, `web-ui/lib/rove-state.ts` | Current UI includes approval buttons and reducer tests. |

## Implemented Differently

| Area | Original design | Current implementation | Impact |
|---|---|---|---|
| DI container | `EngineDeps` with `Arc<dyn ...>` dependencies | `Engine` directly owns `Box<dyn ModelClient>`, `ToolRegistry`, `ContextManager`, `Workspace`, config, hooks | Works for single crate, but less close to the documented dependency graph. |
| CLI entry | Planned subcommands: `dump-config`, `sessions`, `index` | Main CLI has oneshot plus `dump-config`, `index`, and `sessions`; `index` and `rove-index` share the same implementation behind the `rag` feature | The main CLI surface now matches the planned station-1 subcommands. |
| M3 RAG availability | `retrieve_code` / `retrieve_docs` tools plus ingestion | Implemented behind Cargo feature `rag`; ingestion is available through `rove index` and the legacy `rove-index` binary | Useful but not always available in default build. |
| M4 MCP transport | JSON-RPC over stdio/SSE | Stdio JSON-RPC only | Mock-server tests cover stdio; SSE transport and broader server config are not implemented. |
| Model providers | OpenAI, Anthropic, Ollama, DeepSeek, routing/fallback | OpenAI-compatible client plus fake model | Anthropic/Ollama/routing/circuit-breaker work remains open. |
| Tool call parsing | Protocol-specific tool-use normalized in model layer | Text parser handles final text or JSON `{ "tool": ..., "args": ... }` | Simpler and testable, but not yet the documented Anthropic/OpenAI tool-use abstraction. |
| Context management | 7-section budget, cache breakpoints, compaction | Deterministic prompt ordering with session summary and trimmed history | Covers early M1/M2 needs, not the full station-5 design. |
| API cancellation/shutdown | Graceful cancellation token tree | API stores a per-job `CancellationToken`, passes it to `Engine::run_with_cancel`, and serves with `with_graceful_shutdown` through a shutdown token | More cooperative than task aborting, but still not the full app-level parent token tree or in-flight job drain. |
| Web delivery | Roadmap recommended independent Next.js project or temporary axum HTML | Independent Next.js workbench proxies to `/api` | Matches the preferred direction more than the historical `GOAL.md` Path B note. |

## Not Yet Implemented

| Gap | Source design | Current missing piece |
|---|---|---|
| REPL mode and slash commands | `docs/06` station 11 | No `rustyline` REPL, `/session`, `/memory`, `/history`, `/cancel`, etc. |
| Cancellation token tree completion | `docs/06` stations 2, 3, 12 | `Engine::run_with_cancel` and API job tokens exist, but there is no app-level parent token, CLI SIGINT/SIGTERM exit-code mapping, ToolContext token, or post-run hook cancellation token. |
| Prompt cache and compaction | `docs/06` station 5 | No cache breakpoint metadata or compact model flow. |
| Durable/session memory stores | `docs/06` station 8 | Working/session summaries exist through snapshots, but no durable `MEMORY.md`, save-memory tools, or memory index. |
| Anthropic/Ollama/DeepSeek providers | `docs/04` M1 and `docs/06` station 4 | Only OpenAI-compatible and fake clients are present. |
| Routing model client and circuit breaker | `docs/06` station 4 | No fallback provider routing, TTFB probe, or three-state circuit breaker. |
| Native OpenAI/Anthropic tool-use normalization | `docs/06` station 6 | Model layer does not emit structured `ModelChunk::ToolUse`; parser remains JSON-text based. |
| Concurrent tool calls | `docs/05` D7 and `docs/06` station 7 | Tool calls execute serially. |
| `request_input` tool | `docs/06` station 7 | Not present. |
| MCP SSE transport | `docs/04` M4 | Stdio transport exists; SSE transport does not. |
| Always-on RAG tool registration | `docs/04` M3 | RAG requires `--features rag`; default build excludes it. |
| Cargo workspace split | `docs/04` M5 optional | Still a single Rust crate plus separate `web-ui` package. |

## Milestone Status

| Milestone | Status | Evidence |
|---|---|---|
| M0 skeleton | Implemented | Workspace detection, streaming engine, CLI oneshot, trace/report tests. |
| M1 core loop | Mostly implemented | Multi-step loop, file/shell tools, approval policy, hooks, state/report, context trimming, CLI fast paths, fake benchmarks/tests. Missing Anthropic provider and richer retry/time-budget behavior. |
| M2 planner | Mostly implemented | Persisted `TaskPlan`, resume-at-step, replanning after failed steps, `RunStream` identity/cancel handle, and cooperative engine/API cancellation. Missing richer long-task controls and the full cancellation token tree. |
| M3 RAG | Partially implemented | `src/tools/rag.rs`, `src/bin/rove-index.rs`, `tests/rag.rs`; feature-gated and deterministic runtime retrieval by default. |
| M4 MCP | Partially implemented | Stdio MCP proxy and mock-server test exist; SSE transport and real GitHub/filesystem server validation remain. |
| M5 HTTP API | Implemented with additions | Job create/events/state/cancel and approval endpoints have integration coverage. |
| M6 Web UI | In progress | Next.js workbench has chat, plan, tools, trace, cancel, and approval controls; still needs broader runtime polish and deployment story. |

## This Turn's Follow-Up Work

To continue hardening the early milestones, this turn adds the missing `rove sessions` CLI surface from the lifecycle design:

- `src/interfaces/cli/args.rs`: adds a `sessions` subcommand.
- `src/interfaces/cli/sessions.rs`: formats and prints resumable task states.
- `src/state/store.rs`: adds `list_task_states()` for newest-first local snapshot listing.
- `tests/cli_sessions.rs` and `tests/e2e.rs`: cover command parsing, formatting, empty output, and store ordering.

The next continuation adds the `dump-config` fast path from the same lifecycle design:

- `src/interfaces/cli/args.rs`: adds a `dump-config` subcommand.
- `src/interfaces/cli/config.rs`: prints the effective runtime config as JSON.
- `src/main.rs`: routes `dump-config` before task, workspace, model, and tool setup.
- `tests/cli_config.rs`: verifies the formatted JSON and confirms API key values are not printed.

This continuation then closes the remaining station-1 CLI entry gap by integrating RAG indexing into the main binary:

- `src/interfaces/cli/args.rs`: adds an `index` subcommand with path, deterministic, and embedding-model options.
- `src/interfaces/cli/index.rs`: shares indexing behavior between `rove index` and `rove-index`.
- `src/bin/rove-index.rs`: delegates to the shared CLI index module.
- `tests/cli_index.rs`: covers output formatting, default-build feature messaging, and feature-enabled deterministic ingestion.

The next continuation starts the station-2/3/12 cancellation gap without changing the public stream shape yet:

- `Cargo.toml`: adds a direct `tokio-util` dependency for `CancellationToken`.
- `src/core/engine.rs`: adds `run_with_cancel` and checks cancellation around planner, model, approval, and tool waits.
- `src/interfaces/api/mod.rs`: gives each API job a cancellation token and lets the engine emit the cancelled terminal event.
- `tests/e2e.rs`: covers cancellation while a tool future is still pending.

This continuation then adds the station-2 stream handle shape:

- `src/core/engine.rs`: adds `RunStream`, with ID accessors, `cancel()`, `Stream` implementation, and drop-to-cancel behavior.
- `tests/e2e.rs`: covers immediate ID access and handle-driven cancellation during a pending tool call.

This continuation also closes the direct API graceful-shutdown gap:

- `src/interfaces/api/mod.rs`: adds `serve_listener` / `serve_with_shutdown` and wires `serve` through a Ctrl+C-driven shutdown token.
- `tests/api.rs`: covers token-triggered graceful server shutdown and keeps longer async-job polling diagnostics for Windows all-features runs.
