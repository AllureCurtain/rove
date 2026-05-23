# Implementation Comparison - 2026-05-23

This compares the current `main` implementation with the original design in:

- `docs/04-架构与路线图.md`
- `docs/05-下一步-统一执行内核.md`
- `docs/06-请求生命周期.md`

## Summary

`main` is already beyond the original M0-M2 kernel plan and includes partial M3, M4, M5, and M6 surfaces. The largest difference is shape: the original docs describe an eventual `EngineDeps` dependency graph and richer provider routing, while the current implementation remains a simpler single-crate engine. `RunStream`, cooperative cancellation, CLI signal cancellation, ToolContext cancellation propagation, ToolContext input providers, engine-level input-provider wiring, CLI stdin `request_input`, API pending-input `request_input` transport, post-run hook cancellation, prompt-time durable memory, prompt-time session memory file loading, default post-run session summary writes, and a basic routing model client with fallback model-id config, first-chunk probing, in-memory circuit state, and configurable health policy now exist, but the full app-level token tree, layered memory store, durable-memory promotion hooks, user-configured post-run hooks, provider-specific fallback construction, and web input-response UI are still open.

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
| ToolContext input provider | Tools can receive an optional interface-backed input provider | `src/core/types.rs`, `src/core/engine.rs`, `src/tools/traits.rs`, `src/tools/registry.rs`, `src/tools/request_input.rs`, `src/interfaces/cli/input.rs`, `src/interfaces/api/mod.rs`, `src/main.rs`, `tests/request_input_tool.rs`, `tests/e2e.rs`, `tests/api.rs` | The `request_input` tool returns provider answers when one is supplied; `Engine::with_input_provider` passes providers into tool contexts; CLI construction supplies a stdin provider; API construction supplies a pending-input provider. |
| Post-run hook boundary | `RunCompleted` is yielded before post-run hooks, and hooks receive the run cancellation token | `src/hooks/mod.rs`, `src/core/engine.rs`, `tests/e2e.rs` | `HookRegistry` supports post-run hooks with run/workspace/result context; the stream closes after hooks finish, cancellation interrupts them, or per-hook timeout/panic isolation advances to the next hook. |
| Trace/state/report artifacts | `.rove/runs/<run_id>/{trace.jsonl,task_state.json,report.json}` | `src/state/trace.rs`, `src/state/store.rs`, `src/state/artifacts.rs`, `tests/e2e.rs`, `tests/api.rs` | Snapshot writes are schema-versioned; report includes workspace and identity metadata. |
| Tool pipeline | schema -> validate -> hooks -> approval/boundary -> exec -> post-hook | `src/core/executor.rs`, `src/core/boundary.rs`, `src/hooks/mod.rs` | Implemented with pre/post hooks and destructive-tool approval policy. |
| File and shell tools | Built-in workspace tools with boundary checks | `src/tools/fs.rs`, `src/tools/shell.rs`, `tests/e2e.rs` | File tools stay inside workspace; shell rejects empty/NUL commands and destructive calls obey policy. |
| Durable memory tools | `save_memory`, `update_memory_index`, and `read_memory_topic` operate on `.rove/memory/` | `src/tools/memory.rs`, `tests/memory_tool.rs`, `tests/api.rs` | Adds YAML-frontmatter topic files, capped index rebuilding, constrained topic reads, unsafe-topic rejection, and CLI/API registration. |
| Prompt-time durable memory | `MEMORY.md` is read synchronously and injected into context | `src/memory/durable.rs`, `src/core/engine.rs`, `src/core/context.rs`, `tests/e2e.rs` | Missing index files are ignored; present indexes are capped at 200 lines / 25KB before prompt assembly. |
| Prompt-time session memory | `.rove/memory/sessions/<session_id>.md` is read synchronously and injected into context | `src/memory/session.rs`, `src/core/engine.rs`, `tests/e2e.rs` | Missing files are ignored; resume-state summaries still take precedence for resumed runs. |
| Post-run session memory | Default post-run hook writes final output back to `.rove/memory/sessions/<session_id>.md` | `src/hooks/session_memory.rs`, `src/memory/session.rs`, `src/core/engine.rs`, `tests/e2e.rs` | Gives the prompt-time session loader an automatic writer without adding the full `MemoryStore` abstraction yet. |
| Resume snapshots | `task_state.json` supports resume | `src/state/store.rs`, `src/interfaces/cli/oneshot.rs`, `tests/e2e.rs` | Supports `--resume latest` and `--resume <run_id>`. |
| CLI fast paths | Lightweight subcommands avoid full engine setup | `src/interfaces/cli/config.rs`, `src/interfaces/cli/index.rs`, `src/interfaces/cli/sessions.rs`, `src/main.rs`, `tests/cli_config.rs`, `tests/cli_index.rs`, `tests/cli_sessions.rs` | `dump-config`, `index`, and `sessions` now run before workspace/model/tool setup. |
| Planner | Persisted plan and step events | `src/core/planner.rs`, `src/core/engine.rs`, `prompts/planner.md`, `tests/e2e.rs` | Includes replanning after failed planned steps. |
| M5 API | Axum jobs API with SSE and cancel | `src/interfaces/api/mod.rs`, `src/bin/rove-api.rs`, `tests/api.rs` | Implements `POST /jobs`, `GET /jobs/{id}/events`, `GET /jobs/{id}/state`, `POST /jobs/{id}/cancel`, and pending input/approval response endpoints. |
| API approval flow | Pending destructive-tool approval over HTTP | `src/interfaces/api/mod.rs`, `tests/api.rs` | Adds `POST /jobs/{id}/approvals/{call_id}`, which is beyond the original M5 baseline. |
| API `request_input` flow | Pending user input over HTTP | `src/interfaces/api/mod.rs`, `tests/api.rs` | `JobStateResponse` exposes `pending_inputs`, and `POST /jobs/{id}/inputs/{input_id}` resumes the waiting `request_input` tool call. |
| Web workbench | Next.js workbench consumes SSE and shows plan/tools/trace | `web-ui/app/page.tsx`, `web-ui/components/rove-workbench.tsx`, `web-ui/lib/rove-state.ts` | Current UI includes approval buttons and reducer tests. |

## Implemented Differently

| Area | Original design | Current implementation | Impact |
|---|---|---|---|
| DI container | `EngineDeps` with `Arc<dyn ...>` dependencies | `Engine` directly owns `Box<dyn ModelClient>`, `ToolRegistry`, `ContextManager`, `Workspace`, config, hooks | Works for single crate, but less close to the documented dependency graph. |
| CLI entry | Planned subcommands: `dump-config`, `sessions`, `index` | Main CLI has oneshot plus `dump-config`, `index`, and `sessions`; `index` and `rove-index` share the same implementation behind the `rag` feature | The main CLI surface now matches the planned station-1 subcommands. |
| M3 RAG availability | `retrieve_code` / `retrieve_docs` tools plus ingestion | Tool schemas are always registered; default builds return a clear `rag` feature-required message, while real ingestion/retrieval remains behind Cargo feature `rag` | The LLM-visible tool surface is stable in default builds, but real retrieval still requires a RAG-enabled binary. |
| M4 MCP transport | JSON-RPC over stdio/SSE | Stdio JSON-RPC only | Mock-server tests cover stdio; SSE transport and broader server config are not implemented. |
| Model providers | OpenAI, Anthropic, Ollama, DeepSeek, routing/fallback | OpenAI-compatible client plus fake model | Anthropic/Ollama providers and provider-specific fallback construction remain open. |
| Routing model client | Provider fallback with TTFB probe and circuit breaker | `src/models/routing.rs` adds a reusable fallback client with configurable first-chunk probe timeout and in-memory CLOSED/OPEN/HALF_OPEN circuit state; `ROVE_FALLBACK_MODELS`, `ROVE_ROUTING_FAILURE_THRESHOLD`, and `ROVE_ROUTING_OPEN_COOLDOWN_MS` wire fallback model IDs and health policy into non-fake CLI/API OpenAI-compatible model construction | Failover is only attempted before the first response chunk is emitted; partial streams surface their original error instead of switching providers mid-response. Provider-specific fallback construction remains open. |
| Tool call parsing | Protocol-specific tool-use normalized in model layer | Text parser handles final text or JSON `{ "tool": ..., "args": ... }` | Simpler and testable, but not yet the documented Anthropic/OpenAI tool-use abstraction. |
| `request_input` flow | `request_input` asks the user via `ToolContext` and returns the answer | Tool schema is registered in CLI/API; direct tool execution, engine runs configured with `Engine::with_input_provider`, CLI runs, and API jobs can return interface-provided answers | Gives the LLM, engine/tool boundary, CLI, and API a stable response contract; web UI prompt display/submission remains open. |
| Context management | 7-section budget, cache breakpoints, compaction | Deterministic prompt ordering with session summary and trimmed history | Covers early M1/M2 needs, not the full station-5 design. |
| API cancellation/shutdown | Graceful cancellation token tree | API stores an API shutdown token, gives jobs child tokens, passes them to `Engine::run_with_cancel`, and serves with `with_graceful_shutdown` | API shutdown now cancels pending jobs and clears approvals; still missing explicit job-broker drain semantics and the fuller app-level token container from the docs. |
| Web delivery | Roadmap recommended independent Next.js project or temporary axum HTML | Independent Next.js workbench proxies to `/api` | Matches the preferred direction more than the historical `GOAL.md` Path B note. |

## Not Yet Implemented

| Gap | Source design | Current missing piece |
|---|---|---|
| REPL mode and slash commands | `docs/06` station 11 | No `rustyline` REPL, `/session`, `/memory`, `/history`, `/cancel`, etc. |
| Cancellation token tree completion | `docs/06` stations 2, 3, 12 | `Engine::run_with_cancel`, `RunStream::cancel`, CLI signal cancellation, ToolContext token propagation, post-run hook cancellation, and API parent/child job tokens exist. Remaining gaps: explicit app-level runtime token object, REPL-specific cancellation behavior, and panic-hook trace fallback. |
| Prompt cache and compaction | `docs/06` station 5 | No cache breakpoint metadata or compact model flow. |
| Durable/session memory stores | `docs/06` station 8 | Durable topic files, capped `MEMORY.md`, prompt-time durable/session memory loading, default post-run session summary writes, `save_memory`, `update_memory_index`, and `read_memory_topic` now exist. Remaining gaps: full `LayeredMemory`/`MemoryStore`, richer session compaction/promotions, and relevant-memory retrieval. |
| Anthropic/Ollama/DeepSeek providers | `docs/04` M1 and `docs/06` station 4 | Only OpenAI-compatible and fake clients are present. |
| Routing config, probes, and circuit breaker | `docs/06` station 4 | A basic `RoutingModelClient` exists, `ROVE_FALLBACK_MODELS` routes non-fake CLI/API OpenAI-compatible clients through it, first-chunk probe timeouts can fall through to the next candidate, repeated pre-commit failures open an in-memory circuit that later half-opens after cooldown, and routing health threshold/cooldown are configurable. Missing: provider-specific fallback construction. |
| Native OpenAI/Anthropic tool-use normalization | `docs/06` station 6 | Model layer does not emit structured `ModelChunk::ToolUse`; parser remains JSON-text based. |
| Concurrent tool calls | `docs/05` D7 and `docs/06` station 7 | Tool calls execute serially. |
| `request_input` interactive flow | `docs/06` station 7 | Tool surface, fallback output, `ToolContext` input provider trait, provider-backed direct tool answers, engine provider wiring, CLI stdin prompting, and API pending-input response transport exist. Missing: web UI pending-input display/submission and broader interaction polish. |
| MCP SSE transport | `docs/04` M4 | Stdio transport exists; SSE transport does not. |
| Post-run hook hardening | `docs/06` station 10 | Core hook boundary, cancellation, per-hook timeout, panic isolation, and a built-in session summary hook exist. Remaining gaps: durable-memory promotion hooks and user-configured hooks. |
| Cargo workspace split | `docs/04` M5 optional | Still a single Rust crate plus separate `web-ui` package. |

## Milestone Status

| Milestone | Status | Evidence |
|---|---|---|
| M0 skeleton | Implemented | Workspace detection, streaming engine, CLI oneshot, trace/report tests. |
| M1 core loop | Mostly implemented | Multi-step loop, file/shell tools, approval policy, hooks, state/report, context trimming, CLI fast paths, basic routing client with first-chunk probing and configurable circuit state, fallback model-id config, fake benchmarks/tests. Missing Anthropic provider, provider-specific fallback construction, and richer retry/time-budget behavior. |
| M2 planner | Mostly implemented | Persisted `TaskPlan`, resume-at-step, replanning after failed steps, `RunStream` identity/cancel handle, cooperative engine/CLI/API cancellation, ToolContext and post-run cancellation propagation, post-run timeout/panic isolation, durable-memory tools, prompt-time session memory loading, and default post-run session summary writes. Missing richer long-task controls and the full layered memory store. |
| M3 RAG | Partially implemented | `src/tools/rag.rs`, `src/tools/rag_stub.rs`, `src/bin/rove-index.rs`, `tests/rag.rs`, `tests/rag_default.rs`; tool schemas are always present, but real indexing/retrieval remains feature-gated. |
| M4 MCP | Partially implemented | Stdio MCP proxy and mock-server test exist; SSE transport and real GitHub/filesystem server validation remain. |
| M5 HTTP API | Implemented with additions | Job create/events/state/cancel, approval endpoints, and pending-input response endpoints have integration coverage. |
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

This continuation makes the RAG tool surface always registered:

- `src/tools/rag_stub.rs` and `src/tools/mod.rs`: expose default-build `retrieve_code` / `retrieve_docs` stubs with the same schemas as the feature-enabled tools.
- `src/main.rs` and `src/interfaces/api/mod.rs`: register RAG tools unconditionally.
- `tests/rag_default.rs` and `tests/api.rs`: cover default-build schemas, feature-required output, and API job registration.

This continuation wires the station-10 post-run hook boundary:

- `src/hooks/mod.rs`: adds `PostRunHook`, `PostRunHookContext`, post-run registration, and cancellation-aware hook execution.
- `src/core/engine.rs`: routes terminal `RunCompleted` paths through post-run hooks after yielding the terminal event and before stream close.
- `tests/e2e.rs`: covers completed-run context and cancelling a pending post-run hook before the stream closes.

This continuation hardens the station-10 post-run hook runner:

- `src/hooks/mod.rs`: adds per-hook timeout defaults and catches hook panics without stopping later hooks.
- `tests/e2e.rs`: covers timed-out and panicking post-run hooks continuing to subsequent hooks and closing the stream.

This continuation starts the station-7 `request_input` surface:

- `src/tools/request_input.rs` and `src/tools/mod.rs`: add the `request_input` tool schema, prompt validation, and a clear interactive-provider-required fallback.
- `src/main.rs` and `src/interfaces/api/mod.rs`: register `request_input` for CLI and API engine runs.
- `tests/request_input_tool.rs` and `tests/api.rs`: cover schema/fallback behavior and API job registration.

This continuation starts station-8 session memory file loading:

- `src/memory/session.rs` and `src/memory/mod.rs`: add synchronous `.rove/memory/sessions/<session_id>.md` loading for prompt construction.
- `src/core/engine.rs`: injects the loaded session memory when no resume-state summary is present.
- `tests/e2e.rs`: covers prompt inclusion for matching session memory files.

This continuation wires a default station-8/station-10 session memory writer:

- `src/hooks/session_memory.rs` and `src/hooks/mod.rs`: add a default post-run hook that writes final run output as a bounded session summary.
- `src/memory/session.rs`: adds synchronous session summary writing beside the prompt-time reader.
- `src/core/engine.rs`: installs default post-run hooks for standard engine construction while preserving explicit `with_hooks(...)` overrides.
- `tests/e2e.rs`: covers `.rove/memory/sessions/<session_id>.md` persistence after a successful default engine run.

This continuation adds a station-4 routing model client surface:

- `src/models/routing.rs` and `src/models/mod.rs`: add a composable `RoutingModelClient` that tries fallback providers only when the active provider fails before streaming chunks.
- `src/models/routing.rs`: covers successful fallback and the no-mid-stream-fallback guard with unit tests.
- Remaining station-4 gaps are config wiring, provider construction, TTFB probing, and circuit-breaker state.

This continuation advances the station-7 `request_input` provider boundary:

- `src/core/types.rs`: adds `UserInputRequest`, `UserInputProvider`, and optional `ToolContext.input_provider`.
- `src/tools/traits.rs`, `src/tools/registry.rs`, and `src/core/executor.rs`: thread `ToolContext` through tool execution.
- `src/tools/request_input.rs`: returns provider answers when available and keeps the existing provider-required fallback otherwise.
- `tests/request_input_tool.rs`: covers provider-backed answer return in addition to schema and fallback behavior.

This continuation wires `request_input` providers into engine runs:

- `src/core/engine.rs`: adds `Engine::with_input_provider(...)` and passes the configured provider into both tool-call paths.
- `tests/e2e.rs`: covers an actual `request_input` tool call receiving an engine-supplied provider answer and continuing to final output.
- Remaining station-7 gaps are CLI/API provider implementations and user-response transport.

This continuation wires `request_input` into CLI stdin:

- `src/interfaces/cli/input.rs`: adds `StdinInputProvider`, `stdin_input_provider()`, and a tested prompt helper that strips line endings from user answers.
- `src/interfaces/cli/mod.rs` and `src/main.rs`: expose the CLI input module and attach the stdin provider to normal CLI engine construction.
- That slice left API/web response transport for the next station-7 work.

This continuation adds API `request_input` response transport:

- `src/interfaces/api/mod.rs`: adds pending input storage, exposes pending prompts through `JobStateResponse`, installs an API-backed `UserInputProvider`, and adds `POST /jobs/{id}/inputs/{input_id}` to resume waiting tool calls.
- `tests/api.rs`: covers a fake-raw `request_input` job waiting for input, receiving an HTTP answer, finishing successfully, and streaming the completed tool call output.
- Remaining station-7 gap is web UI pending-input display/submission and interaction polish.

This continuation starts station-4 routing config wiring:

- `src/config.rs` and `src/interfaces/cli/config.rs`: add comma-separated `ROVE_FALLBACK_MODELS` parsing and expose fallback model IDs in `dump-config` without leaking secrets.
- `src/models/factory.rs`, `src/main.rs`, and `src/interfaces/api/mod.rs`: build non-fake OpenAI-compatible clients through `RoutingModelClient` when fallback model IDs are configured.
- `tests/model_factory.rs`, `tests/cli_config.rs`, and `src/config.rs`: cover routing model-id construction, config output, and fallback-list parsing.
- That slice left provider-specific fallback config, TTFB probing, and three-state circuit breaker state for later station-4 work.

This continuation adds the first station-4 TTFB probe behavior:

- `src/models/routing.rs`: adds a default 60s first-chunk probe timeout and a test override for routing tests.
- `src/models/routing.rs`: falls through to the next candidate if a provider times out before emitting its first chunk, while preserving the existing no-fallback-after-streaming guard.
- That slice left provider-specific fallback config and three-state circuit breaker state for later station-4 work.

This continuation adds the first station-4 circuit-breaker behavior:

- `src/models/routing.rs`: adds an in-memory `HealthConfig` / health store with CLOSED, OPEN, and HALF_OPEN state.
- `src/models/routing.rs`: marks pre-commit provider failures against circuit health, skips open circuits before cooldown, allows a single half-open probe after cooldown, and closes the circuit again on successful first chunk.
- That slice left provider-specific fallback construction and user-facing health policy configuration.

This continuation wires station-4 routing health policy into config:

- `src/config.rs`: adds `ROVE_ROUTING_FAILURE_THRESHOLD` and `ROVE_ROUTING_OPEN_COOLDOWN_MS` with defaults matching the in-memory routing health store.
- `src/interfaces/cli/config.rs`: exposes the effective routing health policy in `dump-config`.
- `src/models/factory.rs`: passes configured routing health policy into `RoutingModelClient` construction.
- Remaining station-4 gap is provider-specific fallback construction beyond same-base OpenAI-compatible model IDs.
