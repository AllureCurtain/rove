# Runtime Gap Remediation Implementation Plan

> **For implementers:** Execute this plan task by task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the remaining gaps between the original rove design docs and the current implementation, with a focus on runtime correctness, interface parity, safety, recoverability, and maintainability.

**Architecture:** Preserve the current layering: CLI/API/Web are interface shells, `core` owns runtime protocol and execution, providers/tools/memory/state stay behind narrow boundaries. Fix shared contracts first, then improve safety and recovery, then add product-level completeness such as benchmark/eval and richer workspaces. Each phase should leave the project in a working, testable state.

**Tech Stack:** Rust 2024, tokio, axum, serde, rusqlite, reqwest, Next.js, TypeScript, Vitest, cargo test/clippy/fmt, optional `rag` feature.

---

## Purpose And Scope

This document is a repair and implementation guide for the gaps identified by comparing:

- `docs/01-愿景与关键决策.md`
- `docs/04-架构与路线图.md`
- `docs/06-请求生命周期.md`
- `docs/07-产品定位与Workspace.md`
- `docs/runtime/*`
- the current `src/`, `tests/`, and `web-ui/` implementation

The document does not prescribe exact code. It defines the problem, intended design direction, expected result, and acceptance criteria for each workstream so a new Codex Goal session can execute the work incrementally.

This is intentionally broader than a single small patch. A new session should treat this as a backlog and either execute it phase by phase or create one Goal per phase.

## Recommended Execution Order

1. Runtime architecture cleanup: split `engine.rs` and remove planned/unplanned duplication.
2. Tool safety and auditability: add diff/write-set, stronger fs/shell boundaries, and output limits.
3. Interface parity: CLI fast path, planner prompt configuration, API MCP registration, Web auth.
4. Config and memory consistency: make memory paths honor config and remove unused/stub abstractions.
5. Recovery hardening: pending approval/input restart semantics and checkpoint/event consistency.
6. Context and RAG completeness: model-generated compaction and configurable RAG paths/providers.
7. Product verification: benchmark/eval harness and real MCP smoke coverage.
8. Product expansion: future workspace adapters such as `Task`, `Browser`, and `Desktop`.

Do not try to implement all phases in one unreviewed change. Phases 1-4 are the highest value and should be completed before larger feature work.

## Files And Ownership Map

- `src/core/engine.rs`: currently owns too many responsibilities; should be split into focused runtime units.
- `src/core/events.rs`: public runtime event protocol consumed by CLI/API/Web.
- `src/core/planner.rs`: planner prompt loading and plan parsing.
- `src/core/executor.rs`: tool pipeline and hook/permission boundary.
- `src/core/boundary.rs`: shared permission decisions.
- `src/tools/fs.rs`: workspace file path resolution.
- `src/tools/shell.rs`: shell execution policy, timeout, output capture.
- `src/tools/memory.rs`, `src/memory/*`, `src/hooks/session_memory.rs`: memory storage and recall.
- `src/tools/mcp_proxy.rs`: MCP client/proxy and server registration.
- `src/interfaces/api/mod.rs`, `src/interfaces/api/security.rs`: API runtime assembly, job lifecycle, auth.
- `src/interfaces/cli/*`, `src/main.rs`: CLI startup, fast-path commands, rendering.
- `src/state/*`: run artifacts, SQLite index, resume/replay.
- `src/models/*`: provider streams, routing, compaction model usage.
- `src/tools/rag/*`, `src/interfaces/cli/index.rs`, `src/bin/rove-index.rs`: RAG indexing/retrieval/eval.
- `web-ui/lib/rove-client.ts`, `web-ui/lib/rove-state.ts`, `web-ui/lib/rove-types.ts`, `web-ui/components/rove-workbench.tsx`: Web API client, state reducer, event rendering, auth.
- `tests/*`, `web-ui/*.test.ts`: regression and acceptance tests.
- `docs/runtime/*`, `README.md`: current implementation truth and user-facing docs.

---

## Phase 1: Runtime Core Refactor

### 1.1 Split `engine.rs` And Remove Duplicated Execution Logic

**问题所在**

`src/core/engine.rs` is too large and has grown past the original design constraint. It currently has about 1300 lines, while the original design explicitly called out "refuse large files" and an 800-line hard limit. More importantly, planned and unplanned execution paths duplicate model streaming, native tool-use handling, text action parsing, approval handling, tool batch execution, history mutation, and cancellation checks.

This makes future changes risky. Any change to tool execution, approval, provider tool-use, history reconstruction, cancellation, or trace emission must be applied in two branches. Missing one branch can create behavioral divergence that tests may not catch immediately.

**修复及实现思路**

Extract the engine into a small orchestration layer plus focused helper modules:

- `src/core/model_turn.rs`: build model request, consume `ModelEvent`, collect text/tool calls/usage, emit LLM stream events.
- `src/core/tool_turn.rs`: handle one tool call or a batch, including approval-needed event, execution, stable result ordering, history writeback, and tool failure conversion.
- `src/core/run_loop.rs`: unplanned ReAct loop using model turns and tool turns.
- `src/core/plan_loop.rs`: planned loop using the same model/tool turn helpers plus plan-step events and replanning.
- `src/core/engine.rs`: public `Engine`, config, builder methods, and `run_with_cancel` glue.

The key design rule is that both planned and unplanned modes must call the same model-turn and tool-turn functions. Plan-specific behavior should only wrap those shared functions with plan-step lifecycle events.

**预期结果**

The runtime keeps the same external `StreamEvent` protocol and CLI/API/Web behavior, but internal ownership becomes clear. Future changes to tool approval, native provider tool-use, history mutation, and cancellation happen in one place.

**验收标准**

- `src/core/engine.rs` is below 800 lines and mostly contains public API/glue.
- Shared model/tool handling is used by both planned and unplanned paths.
- Existing tests pass without weakening assertions.
- Add at least one regression test proving planned and unplanned runs emit equivalent tool-call lifecycle events for the same model/tool scenario.
- Run:
  - `cargo fmt --all --check`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test --test e2e`
  - `cargo test`

### 1.2 Make Planner Prompt Loading Config-Aware

**问题所在**

`Planner::new()` directly reads `prompts/planner.md` relative to the process working directory. This bypasses `AppConfig`, `Workspace`, and runtime prompt path semantics. It is fragile for API runs started with `-C`, tests that run from a different directory, and future packaging where prompts are not available relative to the process cwd.

**修复及实现思路**

Make planner construction receive a prompt string or a resolved prompt path from interface/runtime wiring. The planner should not decide filesystem paths on its own. Add config support for planner prompt path if needed, or colocate planner prompt loading with the existing system prompt loading boundary.

The engine should hold a planner or planner config that was built from the same resolved workspace/config snapshot as the system prompt.

**预期结果**

CLI and API use the same planner prompt resolution rules. Running from a different process cwd does not silently change planner behavior.

**验收标准**

- `Planner` no longer reads `prompts/planner.md` directly from process cwd.
- A test covers API or engine construction with a custom cwd/config and verifies the configured planner prompt is used.
- Documentation explains where planner prompt configuration lives.
- Existing planner tests still pass.

### 1.3 Make Plan Mutation Semantics Explicit

**问题所在**

The early M2 design required "TODO 可在执行过程中修改". Current implementation can replan after failures, but plan mutation semantics are implicit. It is not clearly defined whether a plan can add/remove/reorder steps during normal progress, how completed steps are preserved, or how plan state is reconciled on resume.

**修复及实现思路**

Define plan mutation as a first-class contract:

- A plan has stable step IDs within one plan revision.
- Replanning creates a new revision while preserving completed work as completed evidence, not by blindly overwriting history.
- A plan update event should distinguish initial plan creation from replanning if the UI or state needs to show it.
- `TaskState.plan` should store enough information to resume without repeating completed steps.

This can be implemented without adding a complex planner framework. The main requirement is consistent semantics and tests.

**预期结果**

Long-running tasks can replan safely after malformed output or tool failure, and resume knows which work is already complete.

**验收标准**

- Tests cover replanning after failure and resuming from the updated plan.
- Completed steps are not repeated after resume unless the model explicitly chooses to redo them.
- Web plan rendering remains coherent after replanning.
- `docs/runtime/implementation-guide.md` describes plan revision semantics.

---

## Phase 2: Tool Pipeline, Safety, And Auditability

### 2.1 Add Diff / Write-Set As A First-Class Pipeline Stage

**问题所在**

The original tool pipeline included `schema -> validate -> input -> pre-hook -> permission -> exec -> post-hook + diff`. Current `Executor` has schema lookup, argument validation, pre-hook, permission, execution, result wrapping, and post-hook. There is no write-set or diff stage. As a result, file mutations are only visible indirectly through tool output or external git diff.

This weakens observability, approval review, resume/debugging, and Web trace value.

**修复及实现思路**

Introduce a tool mutation reporting boundary:

- Tools that can write should return structured metadata describing touched paths and before/after summaries.
- The executor should collect mutation metadata after execution.
- File tools should compute an actual textual diff for UTF-8 file writes.
- Shell should either report "unknown write-set" or optionally run in a mode that snapshots workspace changes around execution.
- `StreamEvent::ToolCallCompleted` or a companion event should expose mutation metadata.
- `RunArtifactRecorder` should persist write-set/diff summaries into `task_state.json` or `report.json`.

Start with `fs_write`, because it has deterministic before/after content. Add shell write-set later if snapshot cost is acceptable.

**预期结果**

Every deterministic file write has an inspectable diff in trace/report artifacts. Users and future Web UI can understand what changed without rerunning external commands.

**验收标准**

- `fs_write` records path, operation type, and before/after or unified diff summary.
- `trace.jsonl` or `report.json` contains mutation metadata for file writes.
- A regression test writes a file twice and asserts the second write records a meaningful diff.
- Existing tool-call output behavior remains compatible.
- Documentation updates the tool pipeline from "missing diff" to "diff/write-set implemented for deterministic file writes; shell write-set policy documented".

### 2.2 Strengthen Filesystem Boundary

**问题所在**

`fs_read` and `fs_write` reject absolute paths and canonicalize the parent, but they do not fully protect against symlink/junction escape scenarios. A path inside the workspace can point to a file outside the workspace through a symlink or platform-specific reparse point.

**修复及实现思路**

Centralize path resolution into a reusable boundary module:

- Treat workspace root as canonical.
- Reject absolute paths.
- Normalize lexical `.` and `..`.
- For reads, canonicalize the final target and ensure it stays under workspace root.
- For writes to existing files, canonicalize the final target and ensure it stays under root.
- For writes to new files, canonicalize the nearest existing ancestor and reject if that ancestor escapes root.
- Add Windows-specific coverage for junction/reparse behavior if practical.

`fs.rs` should not own all security logic; it should call a shared boundary function.

**预期结果**

File tools cannot read or write outside the workspace through path traversal, symlinks, or existing path aliases.

**验收标准**

- Tests cover `../` traversal rejection.
- Tests cover reading through a symlink to a file outside workspace where supported.
- Tests cover writing through an existing symlink path where supported.
- Tests cover writing a new normal file still succeeds.
- `fs_read` and `fs_write` both use the same boundary helper.

### 2.3 Harden Shell Execution

**问题所在**

The shell tool is destructive and approval-gated, but it accepts almost any non-empty command. There is no timeout, output size limit, environment policy, command allow/deny list, or structured exit metadata. A hanging command can hold a run indefinitely until cancellation. Large output can bloat context/history/state.

**修复及实现思路**

Add a shell policy layer configured through `AppConfig`:

- command timeout in milliseconds;
- maximum captured stdout/stderr bytes;
- optional allowlist or denylist;
- environment inheritance policy;
- working directory fixed to workspace root;
- structured output containing exit status, truncated stdout/stderr, and truncation flags.

The default should remain useful for local development, but safe enough for an agent runtime. Approval stays required for destructive shell execution unless policy is `auto`.

**预期结果**

Shell remains useful but bounded. Runs cannot hang forever or emit unbounded output through shell.

**验收标准**

- Test for timeout returns `ToolError::Timeout` or equivalent structured failure.
- Test for large output truncates and marks truncation.
- Test for empty/NUL command still rejects.
- Test for approved short command still succeeds.
- Config dump shows shell policy fields.
- README/runtime docs explain default shell safety behavior.

### 2.4 Make Tool Schema Validation More Complete

**问题所在**

Current validation checks object-ness, required fields, and simple JSON types. It does not enforce enum values, nested objects, array item types, numeric bounds, additional properties, or string constraints. Tool schemas can therefore advertise constraints that are not enforced before execution.

**修复及实现思路**

Either add a small JSON Schema validation crate or implement a consciously limited validator and document its supported subset. Prefer a standard JSON Schema validator if dependency weight is acceptable. If using a limited validator, it must explicitly support the schema features used by built-in tools.

**预期结果**

Tool argument validation matches the schemas rove exposes to models closely enough to prevent invalid inputs before tool execution.

**验收标准**

- `save_memory.type` enum is rejected before tool body execution when invalid.
- Nested object/array validations are covered for at least one tool or test fixture.
- Validation errors preserve `ToolError::InvalidArgs` semantics.
- Documentation states the supported schema subset.

---

## Phase 3: Interface Parity And Runtime Assembly

### 3.1 Restore True CLI Fast Path

**问题所在**

The original lifecycle design required CLI fast paths such as `dump-config` and `sessions` to avoid full runtime initialization. Current `src/main.rs` uses `#[tokio::main]`, so a Tokio runtime is created before any fast-path match executes.

**修复及实现思路**

Split CLI startup into a synchronous `main()` and an async `async_main()`:

- parse args synchronously;
- handle truly synchronous fast paths before creating the runtime;
- create Tokio runtime only for commands that need async work;
- keep behavior and output unchanged.

Some maintenance commands such as `sessions` and `state repair` may still need async work; the key is to be explicit about which commands are true fast paths and which are maintenance async paths.

**预期结果**

Fast commands have fewer side effects and lower startup overhead. The implementation matches the documented lifecycle model.

**验收标准**

- `dump-config` does not require `#[tokio::main]` at process entry.
- CLI behavior is unchanged for normal one-shot runs.
- Add a small test or code-level assertion where practical; at minimum, code structure clearly separates sync fast path from async runtime.
- `docs/runtime/implementation-guide.md` matches the actual startup path.

### 3.2 Register MCP Tools In API Runtime

**问题所在**

CLI loads MCP tools from configured `.rove/mcp_servers.json`. API jobs only register default built-ins and do not load MCP tools. This breaks the "CLI/API/Web consume the same runtime" principle and means Web/API users cannot use configured MCP tools.

**修复及实现思路**

Move runtime tool registry construction into a shared async builder:

- default built-ins;
- configured MCP tools;
- future interface-specific additions only where explicitly justified.

CLI and API should call the same builder. API startup or job creation must handle MCP registration errors clearly. Consider whether MCP clients should be registered once per API process or per job. Process-level registration is more efficient but requires careful lifetime management; per-job registration is simpler but slower.

Recommended first implementation: register MCP tools when building each API engine, matching CLI semantics. Later optimize to process-level cache if needed.

**预期结果**

CLI and API expose the same configured tool surface. Web can use MCP tools through API jobs.

**验收标准**

- API test with a mock MCP server creates a job that calls an MCP tool successfully.
- CLI MCP test continues to pass.
- `default_tool_registry` or replacement builder has one shared path for CLI/API.
- Runtime docs remove any implication that MCP is CLI-only.

### 3.3 Integrate API Token Auth With Web Client

**问题所在**

API bearer token auth exists, but Web client has no way to send Authorization headers. `EventSource` also cannot set custom headers directly in browsers. Therefore, enabling API token auth breaks the Web workbench.

**修复及实现思路**

Use a Next.js proxy route or server-side rewrite layer that can attach Authorization:

- browser talks to `/api/*` without knowing the secret;
- Next.js server reads `ROVE_API_TOKEN` or a similarly named server-only env var;
- proxy forwards `Authorization: Bearer <token>` to the Rust API;
- SSE endpoint must be proxied in a way that preserves streaming.

Do not put the token in public client-side env variables.

**预期结果**

The Web workbench works with token-authenticated local or remote API deployments without exposing the token to browser JavaScript.

**验收标准**

- Web client can create job, stream events, submit approvals, submit inputs, cancel jobs against token-protected API.
- Tests cover request header injection in the proxy/client boundary.
- Documentation explains local unauthenticated mode and token-authenticated mode.
- No token appears in client-side bundle or public env naming.

### 3.4 Align Web Event Types With Runtime Events

**问题所在**

Rust `StreamEvent::LlmMessage` can include structured `tool_calls`, and `ToolCallStarted` includes `tool_use_id`. The Web type definitions currently omit some optional fields. This can silently drop useful information and increases drift risk as runtime events evolve.

**修复及实现思路**

Treat `src/core/events.rs` as the source of truth and keep `web-ui/lib/rove-types.ts` in lockstep:

- add optional fields that exist in Rust events;
- add reducer coverage for fields that affect UI;
- add a test fixture for every event variant.

If event schema changes become frequent, consider generating TypeScript types from Rust schema in a later phase.

**预期结果**

Web can safely parse every current runtime event without losing important metadata.

**验收标准**

- TypeScript `StreamEvent` covers every Rust `StreamEvent` variant and field.
- Reducer tests include all event variants.
- Web typecheck passes.

### 3.5 Add Thinking / Progress Event Semantics

**问题所在**

Original Web acceptance criteria said users should see thinking/tool/progress. Provider adapters normalize `ThinkingDelta`, but Engine currently discards it. Web can show tool and plan progress, but not thinking or a safe equivalent.

**修复及实现思路**

Decide on a product-safe event:

- either expose `thinking_delta` only for providers/models where that is allowed and useful;
- or expose a generic `model_status` / `progress_note` event that avoids showing private reasoning.

Given product safety, prefer `model_status` for now: "model is thinking", "tool-use block started", "waiting for approval", "compacting context", etc. Keep raw chain-of-thought out of the UI unless explicitly designed and allowed.

**预期结果**

The UI has meaningful progress signals without exposing unsafe or provider-restricted reasoning content.

**验收标准**

- Engine emits progress/status events around long model/tool phases.
- Web renders progress in trace/status areas.
- No hidden reasoning text is exposed by default.
- Runtime docs define the policy.

---

## Phase 4: Config And Memory Consistency

### 4.1 Make Memory Config Paths Real

**问题所在**

`AppConfig` exposes `memory.session_dir` and `memory.durable_dir`, validates them, and prints them in `dump-config`. Runtime memory loading and memory tools ignore those fields and derive everything from `workspace.state_dir/memory`.

This is misleading and can break users who configure memory paths expecting them to work.

**修复及实现思路**

Create a `MemoryPaths` or `MemoryConfigResolved` structure during runtime assembly:

- resolved session memory directory;
- resolved durable memory directory;
- recall limit.

Pass this structure to memory loaders, memory tools, and session-memory hook. Avoid having tools infer memory path only from `ToolContext.workspace`.

If passing config through every tool is too invasive, add `RuntimeContext` or extend `ToolContext` with resolved runtime paths.

**预期结果**

Configured memory paths are honored consistently by CLI and API.

**验收标准**

- Test with custom `memory.session_dir` writes session summary to configured path.
- Test with custom `memory.durable_dir` writes durable topic/index to configured path.
- `dump-config` resolved paths match actual writes.
- Runtime docs remove the current caveat about ignored memory path config.

### 4.2 Replace Stub Memory Traits With Actual Runtime Boundaries

**问题所在**

`MemoryStore` and `WorkingMemory` exist but are not the real runtime memory abstraction. Actual memory behavior is implemented through sync helper functions and tools. This creates dead code and conceptual confusion.

**修复及实现思路**

Choose one of two directions:

- Remove unused `MemoryStore` / `WorkingMemory` until a real abstraction is needed.
- Or turn them into the actual abstraction used by Engine.

Recommended first step: remove or quarantine unused stub code, because the current runtime already has concrete session/durable memory functions. Introduce a real trait later only if multiple implementations are needed.

**预期结果**

The memory subsystem contains only code that is either used or clearly part of an active extension point.

**验收标准**

- `#![allow(dead_code)]` is no longer needed because of memory stubs.
- No unused memory abstractions remain in public API unless documented.
- Existing memory tests still pass.

### 4.3 Make Session Summary Quality Explicit

**问题所在**

Session memory is currently written from final output. That is simple, but not necessarily a good summary of what happened. It can miss decisions, files changed, tool results, failures, and user preferences.

**修复及实现思路**

Define a deterministic session summary format first:

- goal;
- final reason;
- final output excerpt;
- completed plan steps;
- tools used;
- files changed once diff/write-set exists.

Later, model-generated summaries can improve this, but deterministic structure should be stable and testable.

**预期结果**

Session memory is useful for resume and future prompts, not just a copy of the final response.

**验收标准**

- Session summary includes goal, status, output summary, and tool/write-set summary where available.
- Resume prompt includes the improved session summary.
- Tests verify summary content for a run with at least one tool call.

---

## Phase 5: State, Resume, And Recovery Hardening

### 5.1 Decide And Implement Pending Approval/Input Restart Semantics

**问题所在**

SQLite has schema slots for pending approvals and inputs, but active channels are live-only and not reconstructed after restart. Current docs say this is intentional. However, the original product promise is "state/resume is first-class", so pending human interaction needs a clearer story.

**修复及实现思路**

Pick one explicit policy:

- **Policy A: Non-reconstructable by design.** On restart, any job waiting for approval/input becomes `interrupted`, and the user must resume the task as a new run.
- **Policy B: Reconstruct pending interaction.** Persist pending requests and rebuild job state so user can answer after restart.

Recommended first implementation: Policy A, because reconstructing channels safely is more complex. Make it explicit and user-visible. Later implement Policy B if the product needs true long-lived human-in-the-loop jobs.

Under Policy A:

- Persist pending approval/input records when they are created.
- Mark them cancelled/interrupted when process restarts.
- Show clear historical state in `/jobs/{id}/state`.
- Resume should include enough context for the model to continue after the interrupted point.

**预期结果**

There is no ambiguous "pending but impossible to answer" state after restart.

**验收标准**

- API restart marks running/pending jobs as `interrupted`.
- Pending approval/input lists are empty after restart, with status explaining interruption.
- Resume latest from an interrupted run works and creates a new run.
- Docs explicitly state the chosen policy.

### 5.2 Persist Event Sequence In Prompt Checkpoints

**问题所在**

`PromptCheckpoint` has `last_event_seq`, but current checkpoint creation sets it to `None`. This weakens precise resume/replay correlation.

**修复及实现思路**

Thread the current trace/event sequence into `RunArtifactRecorder`:

- when trace append returns or determines sequence, pass it to recorder;
- record latest sequence in checkpoint;
- ensure SQLite and trace file sequence agree.

If current trace writer cannot return sequence cleanly, add a small event recording abstraction that writes trace and index once and returns the sequence.

**预期结果**

Prompt checkpoints can point to the exact trace event boundary they summarize.

**验收标准**

- `task_state.json.checkpoint.last_event_seq` is populated after events are recorded.
- The value matches SQLite event replay high-water mark for that run.
- Tests cover at least one multi-event run.

### 5.3 Improve Atomicity Around Trace, SQLite, And Artifacts

**问题所在**

Trace events, SQLite event rows, task state snapshots, and report updates are written through separate paths. Failures can create partial state. The current design accepts files as readable artifacts and SQLite as index, but repair behavior should be more deliberate.

**修复及实现思路**

Define state consistency rules:

- trace file is append-only source of event history;
- SQLite can be rebuilt from trace/task/report files;
- task state is latest resumable snapshot;
- report is final aggregate.

Then improve repair:

- `state repair` imports task states and reconstructs missing event rows from `trace.jsonl`;
- detect mismatched run/report/task identities;
- log or report corrupted artifacts.

**预期结果**

Local state can be inspected and repaired after partial writes.

**验收标准**

- Deleting SQLite and running `rove state repair` reconstructs sessions/jobs/runs/events/task metadata from artifacts.
- Corrupted trace lines are reported without crashing the whole repair.
- Tests cover a missing SQLite rebuild scenario.

---

## Phase 6: Context, Compaction, And RAG

### 6.1 Add Model-Generated Compaction Summaries

**问题所在**

Current prompt checkpoint summaries are deterministic and artifact-based. This is safe, but weak for long tasks because it cannot synthesize intent, decisions, or causal state well.

**修复及实现思路**

Add optional model-generated compaction behind config:

- deterministic compaction remains default fallback;
- when soft budget is crossed, call a compaction prompt using the current provider or a configured cheaper provider;
- store summary text plus metadata: model, prompt version, source message count, failures, fallback mode;
- circuit-break compaction after repeated failures and fall back to deterministic summary.

Do not block completion if compaction fails.

**预期结果**

Long sessions produce more useful resume summaries while preserving deterministic fallback reliability.

**验收标准**

- Config can enable/disable model compaction.
- Test with fake model verifies model-generated summary is stored.
- Test with failing compaction model verifies deterministic fallback and circuit metadata.
- Prompt checkpoint includes compaction mode and failure metadata.

### 6.2 Make RAG Paths Honor Config

**问题所在**

RAG artifacts still assume `.rove` under workspace root. This conflicts with configurable `state.state_dir` and `state.sqlite_path`.

**修复及实现思路**

Introduce resolved RAG paths based on `workspace.state_dir` or explicit RAG config:

- LanceDB directory;
- manifest path;
- index log path;
- eval report directory.

CLI indexing, RAG tools, and eval should use the same path resolver.

**预期结果**

RAG artifacts follow configured state location consistently.

**验收标准**

- RAG index run with custom state dir writes manifest/index/eval under that state dir.
- Retrieval tools read from the same configured location.
- Default `.rove` behavior remains unchanged.
- RAG docs no longer list hard-coded path behavior as a gap.

### 6.3 Complete Production Embedding And Rerank Configuration

**问题所在**

RAG has deterministic embeddings and a foundation for routed embedding providers. Full production embedding/provider config and remote rerank integration are not complete.

**修复及实现思路**

Add explicit RAG provider config:

- embedding provider name/model/base/key;
- deterministic mode flag;
- optional rerank provider/model;
- timeout and fallback policy.

Keep deterministic local mode as test/default fallback. Remote rerank should be optional and skipped clearly when not configured.

**预期结果**

RAG can be used in deterministic local mode for tests and configured provider mode for real retrieval quality.

**验收标准**

- Config dump shows redacted RAG provider settings.
- RAG indexing can use deterministic or provider embeddings based on config.
- Retrieval/eval reports record embedder/reranker identity.
- Tests cover deterministic fallback and missing-key behavior.

### 6.4 Replace RAG Stub Messaging With Capability-Aware Tool Registration

**问题所在**

Default builds expose RAG stub tools that explain the feature requirement. This is acceptable for build size, but it means tool availability does not reflect runtime capabilities precisely.

**修复及实现思路**

Decide whether no-feature RAG tools should be registered as stubs or omitted. If stubs remain, make their schema and failure message clearly mark disabled capability. If omitted, ensure prompts do not advertise unavailable RAG tools.

Recommended: keep stubs for discoverability but add capability metadata so interfaces can show "disabled feature" cleanly.

**预期结果**

Users understand whether RAG is available without confusing disabled tools for broken tools.

**验收标准**

- No-feature build behavior is documented and tested.
- Web/API can distinguish disabled RAG tools if exposed through state or schema metadata.
- Feature-enabled behavior is unchanged.

---

## Phase 7: Model Providers And Routing Polish

### 7.1 Normalize Provider Tool-Use And Text Parsing Contracts

**问题所在**

Provider-native tool-use is normalized to `ModelEvent`, while legacy text JSON parsing still exists in `parse_action`. This compatibility is useful, but the contract is unclear: provider-native tool calls and text-parsed tool calls differ in IDs, history replay, and provider formatting.

**修复及实现思路**

Document and test two supported action paths:

- native provider tool-use path: preferred for real providers;
- JSON text action path: compatibility/fake-model path.

Keep both paths, but make conversions explicit in one shared action builder. Avoid duplicating native-vs-text behavior in planned and unplanned loops.

**预期结果**

Tool-use behavior is stable across providers, fake tests, and text fallback.

**验收标准**

- Tests cover OpenAI/Anthropic/Ollama formatted history after a native tool call.
- Tests cover fake/text JSON tool call compatibility.
- Planned and unplanned loops share the same action conversion.

### 7.2 Add Provider Retry/Backoff Policy

**问题所在**

Routing supports fallback before committed visible output/tool-use, but detailed retry/backoff policy is thin. Rate limits expose retry-after, but runtime behavior does not fully use it.

**修复及实现思路**

Add provider retry config:

- max attempts per provider before fallback;
- backoff base/max;
- respect retry-after for rate limits;
- never retry auth/context length errors;
- preserve "no fallback after committed output" rule.

**预期结果**

Transient provider failures are handled predictably without duplicating user-visible output or tool calls.

**验收标准**

- Tests cover retryable request failure before fallback.
- Tests cover no retry for auth/context errors.
- Tests cover no fallback after committed text/tool-use.
- Tracing records retry attempts and final outcome.

---

## Phase 8: Web Workbench Completeness

### 8.1 Add Resume And Historical Job Controls To Web

**问题所在**

The Web workbench can create a job and stream active state, but it does not expose the same resume/session workflow as CLI/API. API supports `resume`, but Web request type does not include it and UI has no resume controls.

**修复及实现思路**

Add minimal resume support:

- allow `resume: "latest"` when creating a job;
- optionally list historical jobs/sessions after an API endpoint exists or reuse existing state endpoints if sufficient;
- show active run identity and resumed-from identity.

Start with a simple "Resume latest" control rather than a full session browser.

**预期结果**

Web users can continue an interrupted or previous task without dropping to CLI.

**验收标准**

- Web create job request supports `resume`.
- UI has a clear resume-latest action.
- API test and Web reducer/client tests cover resumed job state.

### 8.2 Add Browser-Level E2E Coverage

**问题所在**

Web has unit/type/build checks but no browser-level end-to-end tests. For an SSE workbench, unit tests alone do not prove EventSource, streaming state, approval buttons, and layout behavior work together.

**修复及实现思路**

Add Playwright or equivalent E2E tests:

- start mocked or real local API;
- load workbench;
- create fake job;
- stream events;
- submit approval/input;
- cancel job.

Keep this separate from default fast CI if runtime cost is high.

**预期结果**

The Web workbench has confidence at the browser interaction level.

**验收标准**

- At least one E2E test covers create job -> receive event -> completed state.
- One E2E test covers pending approval interaction.
- CI has either optional Web E2E workflow or documented local command.

---

## Phase 9: MCP Compatibility

### 9.1 Add Real MCP Server Smoke Tests

**问题所在**

MCP tests use a Python mock server. This verifies rove's basic JSON-RPC/proxy behavior but not compatibility with real MCP servers such as filesystem, GitHub, or database servers.

**修复及实现思路**

Add optional smoke tests that can run when dependencies/secrets are available:

- filesystem MCP with temporary directory;
- one public/no-secret MCP server if available;
- GitHub MCP behind env token and ignored by default.

Do not make secret-dependent tests required in normal CI.

**预期结果**

Claims about MCP compatibility are backed by real server smoke coverage without making default CI brittle.

**验收标准**

- Mock MCP tests remain default.
- At least one real stdio MCP smoke test exists behind an explicit env gate.
- Documentation lists tested MCP server types and how to run smoke tests.

### 9.2 Harden MCP Transport Behavior

**问题所在**

MCP transport currently assumes simple line-based JSON-RPC for stdio and has limited timeout/cancellation/error policy. SSE transport coverage is thinner than stdio.

**修复及实现思路**

Add transport policies:

- initialize/list/call timeout;
- child process cleanup;
- stderr capture policy;
- structured MCP error mapping into `ToolError`;
- SSE reconnect or failure policy.

**预期结果**

MCP failures are bounded and understandable to users and logs.

**验收标准**

- Timeout tests for stdio server that does not respond.
- Tool call error from MCP maps to structured `ToolError::ExecutionFailed` or a more specific variant.
- Dropped process is cleaned up.
- Runtime docs describe MCP limitations.

---

## Phase 10: Benchmark And Evaluation Harness

### 10.1 Build General Agent Benchmark Harness

**问题所在**

Original M1 acceptance required `FakeModelClient + 第一个 benchmark 任务` and 3-5 benchmark tasks. Current project has strong tests and RAG eval, but no general agent benchmark harness for behavior across tasks.

**修复及实现思路**

Create a small local benchmark framework:

- benchmark task definition file;
- initial workspace fixture setup;
- scripted fake model or provider model config;
- expected filesystem/state/report assertions;
- summary report output.

Start with fake-model deterministic tasks:

- echo/smoke task;
- read directory and summarize;
- write a file;
- repair a failing test fixture;
- resume from interrupted state.

Avoid depending on external API keys for default benchmark.

**预期结果**

The project can prove core agent behaviors beyond unit tests and can track regressions over time.

**验收标准**

- `cargo test` or a separate `cargo run --bin rove-bench` can run deterministic benchmarks.
- At least 3 benchmark tasks pass locally without network credentials.
- Benchmark output records pass/fail and artifact paths.
- README/runtime docs explain how to run benchmarks.

### 10.2 Add Regression Gates For Original Milestones

**问题所在**

The original docs define milestone acceptance criteria, but the project does not have a single view that maps each acceptance criterion to a test or command.

**修复及实现思路**

Create `docs/runtime/acceptance-matrix.md`:

- M0-M6 criterion;
- current implementation status;
- verification command/test;
- gap owner phase from this plan.

Keep it updated as gaps close.

**预期结果**

Future work can quickly answer "what proves this milestone is done?"

**验收标准**

- Acceptance matrix exists and references concrete tests/commands.
- Matrix has no vague entries such as "manual" unless there is a reason and a checklist.
- Runtime docs link to the matrix.

---

## Phase 11: Workspace Product Expansion

### 11.1 Add `Task` Workspace

**问题所在**

The product positioning says rove is a general workspace runtime, not only a repo/code agent. Current implementation supports only `Folder` and `Repo`.

**修复及实现思路**

Add `WorkspaceKind::Task` as a first expansion:

- creates an isolated local task workspace under configured state or user-specified base;
- has its own `.rove`/state directory;
- supports file tools and memory like Folder;
- can be created from CLI/API without requiring the user to start inside an existing project.

Do not start with Browser/Desktop because they require much more surface area.

**预期结果**

rove can run standalone tasks in an isolated workspace, proving the Workspace abstraction is broader than repos.

**验收标准**

- CLI/API can create a task workspace.
- State and memory are scoped to the task workspace.
- Existing Folder/Repo detection remains unchanged.
- Docs explain Task workspace lifecycle and cleanup.

### 11.2 Define Browser/Desktop Workspace As Future Specs

**问题所在**

Browser/Desktop workspaces are listed in product positioning but have no implementation design. Implementing them without a dedicated spec would sprawl across tools, security, UI, and state.

**修复及实现思路**

Write separate specs before implementation:

- Browser Workspace: browser context ownership, navigation tools, screenshots, downloads, safety boundaries.
- Desktop Workspace: OS automation tools, permissions, screenshots, window targeting, local security.

Do not implement these as part of this remediation Goal.

**预期结果**

The project keeps the extensibility story without destabilizing the current runtime.

**验收标准**

- Separate design specs exist before code changes.
- Current code has no half-implemented Browser/Desktop stubs.

---

## Phase 12: Code Hygiene And Documentation

### 12.1 Remove Global Dead Code Allowance

**问题所在**

`src/lib.rs` has `#![allow(dead_code)]`, which can hide unused stubs and partially abandoned abstractions. This is risky in a fast-moving runtime where unused code often signals broken wiring.

**修复及实现思路**

Remove the global allowance after cleaning up unused memory stubs and any other unused public/private code. If a specific item must remain unused for a valid reason, annotate it locally with a comment explaining why.

**预期结果**

Dead code warnings become useful again.

**验收标准**

- `#![allow(dead_code)]` is removed from `src/lib.rs`.
- `cargo clippy --all-targets -- -D warnings` passes.
- Any remaining local dead-code allowances are justified.

### 12.2 Keep Runtime Docs As Source Of Truth

**问题所在**

Older docs are marked historical, while `docs/runtime` is current. As fixes land, docs can drift again unless updates are part of acceptance.

**修复及实现思路**

For every phase:

- update `docs/runtime/implementation-status.md`;
- update `docs/runtime/implementation-guide.md`;
- update README only for user-facing commands or setup;
- do not edit historical docs except to add pointers if necessary.

**预期结果**

New contributors and future Codex sessions can trust `docs/runtime`.

**验收标准**

- Every closed gap is reflected in `implementation-status.md`.
- Known gaps list shrinks or becomes more precise.
- README quick-start remains accurate.

---

## Cross-Phase Verification Baseline

Run this baseline after each major phase unless the phase only changes docs:

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Run Web checks after Web/API contract changes:

```powershell
cd web-ui
npm test
npm run typecheck
npm run build
```

Run RAG checks after RAG changes:

```powershell
cargo check --features rag --bin rove-index
cargo clippy --all-targets --features rag -- -D warnings
cargo test --features rag
```

Run focused checks whenever relevant:

```powershell
cargo test --test e2e
cargo test --test api
cargo test --test mcp
cargo test --test memory_tool
cargo test --test cli_config
```

## Completion Definition

This remediation plan is complete when:

- engine model/tool turn logic is no longer duplicated;
- file and shell tools have bounded, auditable behavior;
- tool writes produce first-class diff/write-set artifacts where deterministic;
- CLI/API/Web expose consistent configured runtime capabilities;
- memory and RAG path configuration is honored;
- restart/resume semantics for pending human interaction are explicit and tested;
- model compaction has deterministic fallback and tests;
- Web works with API token auth;
- general benchmark/eval exists with deterministic no-network tasks;
- runtime docs accurately describe the implemented state and remaining future scope;
- full default, Web, and RAG verification commands pass.

## Suggested Goal Breakdown For Future Sessions

Use separate Codex Goals for better review and lower risk:

1. **Goal A:** Refactor engine model/tool turn handling without behavior changes.
2. **Goal B:** Implement tool diff/write-set and harden fs/shell boundaries.
3. **Goal C:** Fix runtime assembly parity: CLI fast path, planner prompt config, API MCP, Web auth.
4. **Goal D:** Make memory/RAG config paths real and remove dead stubs.
5. **Goal E:** Harden resume/restart state semantics and checkpoint event sequencing.
6. **Goal F:** Add model-generated compaction with deterministic fallback.
7. **Goal G:** Add benchmark/eval harness and acceptance matrix.
8. **Goal H:** Add Task workspace design and implementation.

Start with Goal A unless there is an urgent user-facing bug in another phase.

