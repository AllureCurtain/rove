# Current Implementation Status

This matrix compares the runtime hardening target with the current implementation.

## MVP Status

MVP reached for the local-first single-user runtime. The exact boundary, included capabilities, exclusions, golden paths, and verification baseline are documented in [mvp-definition.md](mvp-definition.md).

| Area | Current status | Remaining gap |
|---|---|---|
| Local-first default | API defaults to `127.0.0.1:8787`; CLI and state default to the workspace. Remote bind requires token auth unless explicitly marked unsafe. | None for the current local-first target. |
| Configuration | Typed config with defaults, `.rove/config.toml`, env, CLI/API overrides, validation, source summary, redacted dump output, planner prompt path, model compaction controls, shell policy fields, routing retry/backoff fields, and RAG provider settings. | More CLI fields could be exposed as explicit overrides over time. |
| Runtime core | `Engine` is public glue; shared model turns, tool turns, unplanned loop, and planned loop live in focused core modules. Planned and unplanned execution share model streaming, native tool-use conversion, approval, batch execution, and history writeback. | None for the current runtime split target. |
| State layer | Files under `.rove/runs/` plus SQLite index for sessions/jobs/runs/events/reports/task states. WAL, foreign keys, migrations, busy timeout, lazy task-state import, TTL cleanup, and explicit `rove state repair/cleanup` commands exist. `state repair` rebuilds task-state metadata, report rows, events, and event offsets from artifacts while reporting corrupted trace lines. | None for the current repair target. |
| API jobs | Live registry stores active handles; SQLite stores durable job/run/event state. Restart marks stale running jobs `interrupted`; historical state and SSE replay read from SQLite. Pending approval/input rows are persisted while live and marked `interrupted` on restart, but answer channels are not reconstructed by design. | True long-lived human-in-the-loop reconstruction remains intentionally out of scope. |
| Interface runtime assembly | CLI uses a sync fast path for `dump-config` before Tokio startup. Running `rove` with no task enters a line-oriented REPL; quoted and unquoted multi-word tasks run as one-shot messages. Optional `rove tui` uses the same `CliRuntime`, `Engine`, canonical events, run artifacts, cancellation token, and approval/input providers through an alternate-screen Ratatui/Crossterm shell. Its session picker resolves persisted state through the shared `StateStore`; the timeline is an in-memory view, not a second event or persistence contract. CLI and API share the async runtime tool registry builder, including configured MCP tools. | The TUI intentionally supports one active session/run at a time. Multi-session tabs, background task management, and mouse interaction remain future product scope. |
| Full-screen TUI MVP | `rove tui` has a reducer-driven EventStream loop, bounded async run projection, a live renderer-neutral chronological timeline, wrapped transcript/history, cancellation, confirmation-before-exit, resize/narrow rendering, bounded session/tool/help overlays, hardened terminal setup/restoration, and capability-gated approval/input modals. The session picker lists only non-running persisted task states and revalidates a selected identity before resume. Non-Windows terminals with keyboard enhancement use direct `Y`/`Enter` actions. Windows uses native key events with a non-text `F8` confirmation/submission boundary because pasted text is not distinguishable from typed text. A non-clone controller opens a modal only after the canonical event and live responder match by kind and ID. Bracketed paste, held-key tracking, and a post-draw arming boundary prevent pre-modal input from resolving it. Cancellation, completion, EOF, draw failure, unsupported event capabilities, and cross-run cleanup fail closed; unsupported terminals reject approval and return a typed unavailable input error without opening a modal. Empty legacy view state uses a sanitized aggregate fallback; live updates use timeline entries. The shared driver drains post-run hooks and finalizes artifacts before publishing completion. | The standard-library real-terminal smoke is opt-in and Unix-only. Windows reports a typed skip because no native ConPTY automation is included; that skip is not a pass. Display sanitization is bounded defense in depth, not a formal guarantee for arbitrary provider text. |
| MCP compatibility | Stdio/SSE MCP tools register through the shared runtime builder. Stdio transport has request timeouts, stderr diagnostic capture, JSON-RPC error mapping, and child cleanup coverage. Default tests use local stdio fixtures, and an env-gated real filesystem MCP smoke test verifies the official stdio server path when enabled. | Broad compatibility with secret-backed servers such as GitHub or databases remains optional smoke scope, not default CI. |
| Status semantics | `init`, `running`, `done`, `error`, `cancelled`, and `interrupted` are represented. | None for the current lifecycle target. |
| Context budgets | Token-estimated context builder with soft, hard, and reserved budgets. | Token counting is approximate, not provider tokenizer based. |
| Prompt checkpoints | `PromptCheckpoint` stores summary, preserved tail, plan, memory pointers, last step, last event seq, token estimate, compacted count, and compaction metadata. Optional model-generated compaction can emit richer summaries; deterministic fallback records degraded/circuit metadata and keeps resume reliable. Resume prefers checkpoint tail/summary. | None for the current compaction target. Provider-tokenizer counts remain approximate under context budgets. |
| Tool orchestration | Tool batches can run parallel when every call is non-destructive and `parallel_safe` and no interactive input provider is installed; interactive runtimes conservatively use the serial event/ack path because tool schemas cannot yet declare dynamic input use. Results are emitted in deterministic call order. Tool calls that depend on previous tool output naturally run in later model turns and therefore serialize. Destructive tools go through approval. Tool args get recursive schema validation for the supported subset. `fs_write` reports diff/write-set metadata into `ToolResult` and reports; shell execution is timeout/output/env/denylist bounded. | No separate batch hook layer or general tool-dependency DAG inference yet. Shell write-sets are not inferred. |
| Provider abstraction | OpenAI-compatible, Anthropic, Ollama, and Fake are peer providers behind `ModelClient`. Stream events are normalized to `ModelEvent`; native tool-use is preferred, and JSON text action parsing remains as a fake/compatibility fallback. | Provider-specific advanced features remain intentionally thin. |
| Routing and fallback | Fallback models and native fallback providers are supported. Fallback happens before committed visible output/tool-use, uses provider-aware target IDs, and shares API health state across jobs. Routed candidates support configured max attempts, base/max backoff, rate-limit retry-after, and no retry/fallback after committed output or tool-use. Structured tracing records probe, retry, and exhaustion outcomes. | None for the current retry/backoff target. |
| Memory layers | Working prompt memory, configured session summary files, configured durable topic files, bounded durable recall, guarded durable promotion through `save_memory`, and deterministic session summaries with goal/status/output/tool/write-set/plan-step metadata. | Durable recall is lightweight lexical relevance, not a full knowledge system. |
| Workspaces | Folder and Repo detection remain unchanged. Task workspaces can be created from CLI or API under a configured/requested base, with config rebased so state, filesystem tools, shell, session memory, and durable memory are scoped to the task root. Browser and Desktop are future design specs only. | Browser/Desktop implementation requires dedicated future plans. |
| RAG | Feature-gated staged RAG pipeline with LanceDB, manifest fallback, deterministic embeddings, provider embedding config, configured state-dir artifact paths, retrieval channels, postprocessing, eval reports with embedder/reranker identity, RAG prompt formatting, lightweight code-aware chunking, capability-aware no-feature stubs, routing embedder support, and optional routed remote rerank for eval retrieval with `rerank-noop` fallback. | Agent tool-time retrieval intentionally remains deterministic today; passing configured embedder/reranker services into runtime tool construction is a later extension. |
| API security | Config includes bind address, token auth, CORS origins, rate limit, and unsafe remote override. Middleware enforces bearer token auth, CORS allowlists, and per-process request limits. | Multi-user identity and distributed rate limiting are later deployment/product scope, not active runtime gaps. |
| Web | Standalone Next.js workbench with tests, typecheck, and build in CI. The server-side `/api/*` proxy can inject `ROVE_API_TOKEN` into upstream Rust API requests while preserving SSE streams. Runtime event types include structured tool-call metadata, tool mutations, prompt compaction events, and safe `model_status` progress events. The workbench can start a resume-latest job, displays active/resumed run identity, and has optional Playwright E2E coverage for create-to-complete, approval, and resume flows. | None for the current Web completeness target. Browser E2E remains optional outside default CI. |
| CI | Default Rust/Web workflow, separate RAG workflow, and scheduled/manual release-gate workflow are split. The release gate runs deterministic local-full integration and can run real-provider gates when configured. | None for the current release-gate target. |
| Benchmark/eval | `rove-bench` runs deterministic no-network benchmark tasks from JSON definitions through the real engine/tool/state paths and reports pass/fail plus artifact paths. | Broader provider-backed or long-running benchmark suites remain optional future scope. |
| Code hygiene | Dead code warnings are enforced by removing the global `#![allow(dead_code)]` from `src/lib.rs`; default clippy runs with `-D warnings`. Runtime maintenance favors clear responsibility boundaries over a hard Rust-file line-count limit. | Local dead-code allowances should remain rare and justified inline. Split large modules when ownership, testability, or reviewability improves. |
| Docs | Runtime docs are the source of truth for current behavior. Root README points new readers to `docs/runtime/`, and runtime docs explain quick start, architecture, subsystems, current-vs-target status, verification, and the M0-M6 acceptance matrix. Older `04/05/06` docs are marked historical. | Keep future architecture updates centered in `docs/runtime/`. |

## Acceptance Criteria Mapping

| Criterion | Status |
|---|---|
| Default running remains local-first. | Met |
| Config has multi-source priority and secret redaction. | Met |
| Planner prompt follows runtime config resolution. | Met |
| Engine model/tool turn handling is shared by planned and unplanned execution. | Met |
| Plan resume preserves completed step state and avoids repeating completed steps. | Met |
| State uses file artifacts plus SQLite index. | Met |
| Resume prefers checkpoint reconstruction. | Met |
| API restart semantics for pending approval/input are explicit and tested. | Met with Policy A: mark interrupted, resume as a new run |
| `state repair` rebuilds SQLite from task, trace, and report artifacts. | Met |
| Context is token-budget and segmented prompt aware. | Met, with approximate token estimates |
| Compaction can automatically trigger with degradation/circuit semantics. | Met, including optional model-generated summaries with deterministic fallback |
| Tool calls support batch parallelism with stable writeback order. | Met |
| Deterministic file writes expose diff/write-set metadata. | Met for `fs_write` |
| File tools reject traversal and symlink/reparse escapes outside the workspace. | Met |
| Shell execution is policy bounded and reports structured output metadata. | Met |
| Tool schema validation rejects invalid enum, nested, array, numeric-bound, and additional-property inputs before execution. | Met |
| Provider layer is unified with native peers. | Met |
| Provider native tool-use and JSON text action paths have one shared conversion boundary. | Met |
| Routed provider retry/backoff policy covers retryable failures, rate limits, non-retryable auth/context errors, and committed-output safety. | Met |
| Memory is working/session/durable with controlled promotion. | Met |
| CLI/API can create Task workspaces with scoped state and memory. | Met |
| Existing Folder/Repo workspace detection remains unchanged. | Met |
| Browser/Desktop workspace specs exist without runtime stubs. | Met |
| RAG artifacts honor configured state paths. | Met |
| RAG provider config supports deterministic fallback and missing-key behavior. | Met |
| RAG remote rerank can reorder eval retrieval chunks and fall back to noop. | Met |
| RAG tool schemas expose capability metadata for enabled and disabled builds. | Met |
| API job/state is durable and live handles are active-only. | Met |
| CLI fast paths avoid async runtime startup where practical. | Met for `dump-config`; no-arg startup now enters the REPL by design |
| `rove tui` reuses the shared engine/events/artifacts; renders the bounded canonical-order timeline; safely selects resumable non-running state; exposes bounded tool/help overlays; handles live approval/input through direct enhanced-terminal actions or the Windows F8 safety path; fails closed otherwise; keeps the full-screen session open after run completion/cancellation; and restores attempted terminal modes when the TUI exits, its loop errors, or panic unwinds. | Met by focused TUI/terminal unit tests, fake-provider artifact coverage, resume-negative tests, timeline ordering/redaction tests, fail-closed interaction tests, and the destructive-tool rejection test. `scripts/tui-pty-smoke.py --run` adds opt-in Unix PTY evidence; Windows ConPTY remains unautomated and returns a typed skip. |
| API jobs expose configured MCP tools. | Met |
| MCP has a real stdio smoke test behind an explicit env gate. | Met |
| MCP transport has timeout, JSON-RPC error mapping, stderr diagnostics, and child cleanup coverage. | Met |
| Web works with token-authenticated API through a server-side proxy. | Met |
| Web can create resume-latest jobs and display resumed run identity. | Met |
| Browser E2E covers create-to-complete and pending approval interaction. | Met |
| Web event types cover Rust `StreamEvent` fields. | Met |
| Safe progress semantics exist without exposing hidden reasoning. | Met through `model_status` |
| Deterministic local benchmark suite exists with at least three no-network tasks. | Met |
| M0-M6 acceptance matrix maps criteria to concrete verification commands. | Met |
| CI covers Rust/Web/RAG in separate layers. | Met |
| Root README explains the project mainline. | Met |
| Dead code warnings are enforced instead of globally allowed. | Met |
| Runtime docs are the source of truth for current behavior. | Met |

## Pico-Inspired Runtime Provider Upgrades

- OpenAI Responses provider: implemented as `openai-responses`, separate from
  `openai-compatible`.
- Runtime loop: documented as Plan outside, ReAct inside.
- Runtime identity: persisted in checkpoints and reports for resume diagnostics.
- Prompt build metadata: recorded in prompt events and run reports.
- Tool execution metadata: recorded for success and failure paths.
- Benchmark evidence: result package format documented under
  `benchmarks/results/`.
