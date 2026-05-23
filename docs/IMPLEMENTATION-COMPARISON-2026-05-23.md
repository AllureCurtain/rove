# Implementation Comparison - 2026-05-23

This compares the current `main` implementation with the original design in:

- `docs/04-架构与路线图.md`
- `docs/05-下一步-统一执行内核.md`
- `docs/06-请求生命周期.md`

## Summary

`main` is already beyond the original M0-M2 kernel plan and includes partial M3, M4, M5, and M6 surfaces. The largest difference is shape: the original docs describe an eventual `EngineDeps` dependency graph and richer provider routing, while the current implementation remains a simpler single-crate engine. `RunStream`, cooperative cancellation, CLI signal cancellation, ToolContext cancellation propagation, and prompt-time durable memory now exist, but the full app-level token tree, layered memory store, and provider-routing design are still open.

## Implemented Close To The Design

| Area | Original design | Current evidence | Notes |
|---|---|---|---|
| Core as library, interfaces as shells | CLI/API/Web consume the same event stream | `src/core/engine.rs`, `src/interfaces/cli/oneshot.rs`, `src/interfaces/api/mod.rs`, `web-ui/lib/rove-client.ts` | Core does not import `interfaces`; CLI/API consume `StreamEvent`. |
| Stream event model | Tagged enum serialized for CLI/SSE/Web | `src/core/events.rs` | Uses `#[serde(tag = "type", rename_all = "snake_case")]`; includes run, LLM, tool, approval, plan, and completion events. |
| Workspace boundary | Folder/Repo detection and `.rove` state root | `src/core/workspace.rs`, `tests/e2e.rs` | Detects nearest git root and stores state under `<root>/.rove`. |
| Run identity | ULID newtypes for session/job/run/call | `src/core/types.rs` | Current `RunRequest` carries explicit `session_id`, `job_id`, and `run_id`. |
| `RunStream` handle | `Engine::run(req) -> RunStream` with IDs and `cancel()` | `src/core/engine.rs`, `tests/e2e.rs` | `RunStream` exposes `session_id()`, `job_id()`, `run_id()`, `cancel()`, and cancels on drop. |
| CLI cancellation | SIGINT/SIGTERM cancels the engine run and maps cancelled exit codes | `src/main.rs`, `src/interfaces/cli/oneshot.rs`, `tests/e2e.rs` | CLI uses `Engine::run_with_cancel`; Ctrl+C exits 130 and Unix SIGTERM exits 143 after cancelled artifacts are finalized. |
| ToolContext cancellation | Active run cancellation token is available at the tool boundary | `src/core/types.rs`, `src/core/engine.rs`, `src/hooks/mod.rs`, `tests/e2e.rs` | Pre/post tool hooks receive the same token through `ToolContext`; tool futures are still interrupted by the engine's select boundary. |
| Trace/state/report artifacts | `.rove/runs/<run_id>/{trace.jsonl,task_state.json,report.json}` | `src/state/trace.rs`, `src/state/store.rs`, `src/state/artifacts.rs`, `tests/e2e.rs`, `tests/api.rs` | Snapshot writes are schema-versioned; report includes workspace and identity metadata. |
| Tool pipeline | schema -> validate -> hooks -> approval/boundary -> exec -> post-hook | `src/core/executor.rs`, `src/core/boundary.rs`, `src/hooks/mod.rs` | Implemented with pre/post hooks and destructive-tool approval policy. |
| File and shell tools | Built-in workspace tools with boundary checks | `src/tools/fs.rs`, `src/tools/shell.rs`, `tests/e2e.rs` | File tools stay inside workspace; shell rejects empty/NUL commands and destructive calls obey policy. |
| Durable memory tools | `save_memory`, `update_memory_index`, and `read_memory_topic` operate on `.rove/memory/` | `src/tools/memory.rs`, `tests/memory_tool.rs`, `tests/api.rs` | Adds YAML-frontmatter topic files, capped index rebuilding, constrained topic reads, unsafe-topic rejection, and CLI/API registration. |
| Prompt-time durable memory | `MEMORY.md` is read synchronously and injected into context | `src/memory/durable.rs`, `src/core/engine.rs`, `src/core/context.rs`, `tests/e2e.rs` | Missing index files are ignored; present indexes are capped at 200 lines / 25KB before prompt assembly. |
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
| API cancellation/shutdown | Graceful cancellation token tree | API stores an API shutdown token, gives jobs child tokens, passes them to `Engine::run_with_cancel`, and serves with `with_graceful_shutdown` | API shutdown now cancels pending jobs and clears approvals; still missing deeper ToolContext/post-run cancellation and explicit job-broker drain semantics. |
| Web delivery | Roadmap recommended independent Next.js project or temporary axum HTML | Independent Next.js workbench proxies to `/api` | Matches the preferred direction more than the historical `GOAL.md` Path B note. |

## Not Yet Implemented

| Gap | Source design | Current missing piece |
|---|---|---|
| REPL mode and slash commands | `docs/06` station 11 | No `rustyline` REPL, `/session`, `/memory`, `/history`, `/cancel`, etc. |
| Cancellation token tree completion | `docs/06` stations 2, 3, 12 | `Engine::run_with_cancel`, `RunStream::cancel`, CLI signal cancellation, ToolContext token propagation, and API parent/child job tokens exist, but post-run hook cancellation token remains open. |
| Prompt cache and compaction | `docs/06` station 5 | No cache breakpoint metadata or compact model flow. |
| Durable/session memory stores | `docs/06` station 8 | Durable topic files, capped `MEMORY.md`, prompt-time index loading, `save_memory`, `update_memory_index`, and `read_memory_topic` now exist. Remaining gaps: full `LayeredMemory`/`MemoryStore`, session memory files, and relevant-memory retrieval. |
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
| M2 planner | Mostly implemented | Persisted `TaskPlan`, resume-at-step, replanning after failed steps, `RunStream` identity/cancel handle, cooperative engine/CLI/API cancellation, ToolContext cancellation propagation, and durable-memory tools. Missing richer long-task controls, post-run cancellation propagation, and the full layered memory store. |
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

This continuation then links API shutdown to job cancellation:

- `src/interfaces/api/mod.rs`: adds `ApiState::with_shutdown`, stores the parent shutdown token, and creates child cancellation tokens for jobs.
- `tests/api.rs`: covers parent-token cancellation of a pending approval job, including approval cleanup and cancelled report artifacts.

This continuation starts the station-8 durable-memory surface:

- `src/tools/memory.rs`: adds the `save_memory` tool, safe topic normalization, YAML-frontmatter topic writes, and capped `MEMORY.md` index rebuilding.
- `src/main.rs` and `src/interfaces/api/mod.rs`: register `save_memory` for CLI and API engine runs.
- `tests/memory_tool.rs` and `tests/api.rs`: cover topic/index writes, unsafe topic rejection, hard index limits, and API job registration.

This continuation fills the remaining explicit station-8 durable-memory tools:

- `src/tools/memory.rs`: adds `update_memory_index` and `read_memory_topic`, reusing the same safe topic boundary and index builder.
- `src/main.rs` and `src/interfaces/api/mod.rs`: register both tools for CLI and API engine runs.
- `tests/memory_tool.rs` and `tests/api.rs`: cover index rebuilding from existing topics, constrained topic reads, unsafe read rejection, and API job registration for both tools.

This continuation wires durable memory into prompt construction:

- `src/memory/durable.rs`: adds synchronous `MEMORY.md` loading with 200-line / 25KB truncation.
- `src/core/context.rs` and `src/core/engine.rs`: inject the loaded durable memory section before history/current task messages.
- `tests/e2e.rs`: covers prompt inclusion and hard-limit enforcement for oversized manual indexes.

This continuation wires cancellation into the CLI oneshot path:

- `src/interfaces/cli/oneshot.rs`: adds `run_oneshot_with_cancel`, returns the terminal reason, and finalizes cancelled artifacts.
- `src/main.rs`: listens for Ctrl+C and Unix SIGTERM, cancels the run token, and maps cancelled exits to 130/143.
- `tests/e2e.rs`: covers pre-cancelled oneshot runs returning `Cancelled` and writing cancelled reports.

This continuation propagates cancellation through `ToolContext`:

- `src/core/types.rs` and `src/core/engine.rs`: add the active `CancellationToken` to `ToolContext` and pass it from each engine tool-call path.
- `tests/e2e.rs` and `tests/memory_tool.rs`: update direct executor contexts and cover pre-tool hooks observing a cancelled token.
- `docs/IMPLEMENTATION-COMPARISON-2026-05-23.md`: narrows the cancellation gap to post-run hook cancellation.
