# CLI REPL And RAG Rerank Design - 2026-05-29

This spec defines the next two implementation phases for `rove`:

1. Add a terminal REPL so running `rove` with no task opens an interactive prompt.
2. Wire remote RAG rerank into the existing retrieval pipeline after the REPL work lands.

This is a design document, not an implementation plan. A future implementation session should turn
this spec into a step-by-step plan before editing production code.

## Suggested Implementation Objective

Use this objective in a new implementation session:

> Based on `docs/design/2026-05-29-cli-repl-and-rag-rerank-design.md`, implement the rove CLI REPL first, preserving one-shot task behavior for `rove "<task>"`, then implement remote RAG rerank behind the existing `rag` feature gate and config fields. Keep existing API/Web behavior stable, avoid Browser/Desktop scope, and verify with focused CLI, RAG, and regression tests.

## Current State

`rove` currently has a one-shot CLI path:

- `src/interfaces/cli/args.rs` uses `clap` for startup arguments and subcommands.
- `src/main.rs` prints help when no message is supplied.
- `src/interfaces/cli/oneshot.rs` consumes the engine event stream, renders output, records artifacts, and exits.
- State is already modeled as `SessionId`, `JobId`, `RunId`, `TaskState`, run artifacts, and a SQLite-backed state index.
- `resolve_resume_state()` supports `--resume latest` or a run id.

`rove` also has a staged RAG implementation behind the `rag` feature:

- RAG ingestion, retrieval channels, query rewrite, postprocessors, and eval reports exist.
- Config fields exist for `rag.rerank_provider`, `rag.rerank_model`, and `rag.rerank_api_key`.
- Retrieval currently wires `NoopRerankPostProcessor`, which truncates results but does not call a rerank model.
- Eval reports currently record `reranker = "none"`.

`D:/Study/project/agent/pico` is the closest local product reference for REPL behavior:

- Running `pico` enters an interactive `pico> ` prompt.
- Running `pico "<task>"` executes once and exits.
- Slash commands are local commands, not model prompts.
- The loop is intentionally simple rather than a full-screen TUI.

`D:/Study/project/agent/ragent` is the closest local architecture reference for rerank:

- Rerank is a model-facing infra capability, not business logic embedded in retrieval.
- `RerankService` provides a stable interface.
- `RoutingRerankService` chooses candidates and reuses model health/fallback.
- `NoopRerankClient` is a fallback.
- `RerankPostProcessor` calls the service at the end of the retrieval postprocessor chain.

## Product Decisions

### CLI Entry Behavior

Adopt this behavior:

```text
rove
# enter REPL

rove "analyze this project"
# execute one-shot and exit

rove index
rove sessions
rove state repair
rove dump-config
# execute subcommand and exit
```

This matches the user's preferred model and the local `pico` reference. It also preserves script and
CI compatibility for one-shot runs.

### Library Choice

Keep `clap` for startup parsing. Add `rustyline` for REPL input.

Do not use `ratatui` in this phase. `ratatui` is for full-screen terminal applications and would
force a larger event/rendering architecture than this feature needs. The first REPL should be a
line-oriented terminal prompt, not a dashboard.

### Development Order

Implement REPL first, rerank second.

The REPL work will require extracting shared CLI runtime construction and stream rendering. Doing
that before rerank gives the later RAG work a cleaner place to expose eval and diagnostics through
the CLI.

## Phase 1: CLI REPL

### Goals

- Running `rove` with no message and no subcommand opens an interactive prompt.
- Running `rove "<task>"` remains one-shot and exits.
- Existing subcommands remain non-interactive.
- REPL runs ordinary user input through the same engine/runtime as one-shot.
- REPL preserves conversation continuity across prompts.
- The implementation reuses the existing event stream renderer and artifact recorder.
- Ctrl+C behavior is useful: at the prompt it should not exit; while a run is active it should cancel only that run and return to the prompt.
- Ctrl+D exits cleanly.

### Non-Goals

- No full-screen TUI.
- No Browser/Desktop workspace scope.
- No multiline editor in the first pass.
- No dynamic `/model` or `/cwd` command in the first pass; these require rebuilding runtime state and should be a later enhancement.
- No change to API/Web job behavior.
- No change to model routing semantics.

### Module Layout

Add or refactor toward this shape:

```text
src/interfaces/cli/
├── args.rs
├── approval.rs
├── config.rs
├── input.rs
├── mod.rs
├── oneshot.rs
├── repl.rs          # new interactive loop
├── runtime.rs       # new shared CLI runtime builder
├── render.rs        # new shared stream renderer/recorder
├── sessions.rs
└── state.rs
```

The exact split can be adjusted during implementation, but the responsibilities should remain clear:

- `args.rs`: startup argument parsing only.
- `runtime.rs`: load config, detect workspace, build model, registry, context manager, engine, state store.
- `render.rs`: consume `RunStream`, print stream events, record artifacts, finalize reports.
- `oneshot.rs`: one-shot orchestration around `runtime.rs` and `render.rs`.
- `repl.rs`: line input loop and slash command dispatch.

`src/main.rs` should become thinner. It should route to subcommands, one-shot, or REPL rather than
owning all runtime construction inline.

### Dependency

Add the dependency through Cargo so the implementation session gets the current compatible release
and updates `Cargo.lock` consistently:

```powershell
cargo add rustyline
```

No extra terminal UI crate is needed.

### Startup Flow

`main` should route as follows:

```text
parse Args with clap

if command == dump-config:
  run sync dump-config and exit

if command == index/sessions/state:
  run subcommand and exit

if message is Some:
  build CLI runtime
  run one-shot
  exit

if message is None:
  build CLI runtime
  run REPL
  exit when user exits
```

The current `Args::is_sync_fast_path()` behavior must change: no-message/no-command is no longer a
help fast path. Only `dump-config` should remain a sync fast path unless another subcommand truly
requires it.

### Runtime State Model

REPL should use one `SessionId` for the lifetime of the REPL process. Each user prompt should create
a new `RunId`. For continuity, each prompt should resume from the latest successful `TaskState` in
that REPL session.

Use this first-pass identity model:

```text
REPL process:
  session_id = new SessionId
  active_resume_state = optional TaskState

For each user prompt:
  job_id = active_resume_state.job_id if resuming an existing task, otherwise new JobId
  run_id = new RunId
  resume_state = active_resume_state
  after run completes, active_resume_state = latest task state for that run
```

This makes the REPL a single continuing task by default. It matches user expectations that follow-up
prompts can refer to earlier prompts. It also fits the existing `TaskState` resume model without
requiring a new conversation table.

Use this alternative only if tests show that reusing one `job_id` corrupts or obscures existing
state-index behavior:

```text
same REPL session_id, new job_id per prompt, resume_state from previous prompt
```

The design preference is same session plus continuity. The exact `job_id` policy should be chosen
based on the least invasive fit with `StateIndex`.

### History

Persist input history under the configured state directory:

```text
<state_dir>/repl_history
```

This lets users use the up/down arrow keys across process restarts. History write failures should be
warnings, not fatal errors.

### Prompt

Use a simple prompt:

```text
rove>
```

Avoid a large banner in the first pass. `pico` has a welcome panel, but `rove` is already a runtime
project with more operational output. A compact startup line is enough:

```text
rove REPL - type /help for commands, /exit to quit
```

### Slash Commands

First pass commands:

```text
/help
/exit
/quit
/clear
/sessions
/resume latest
/resume <run_id>
```

Command behavior:

- `/help`: print local command help.
- `/exit`, `/quit`: exit REPL with status 0.
- `/clear`: clear the terminal if practical; if not, print enough newlines to visually reset.
- `/sessions`: reuse `sessions::format_task_states()` over the current state store.
- `/resume latest`: set `active_resume_state` to the latest task state.
- `/resume <run_id>`: set `active_resume_state` to the specified task state.

Slash commands should not be sent to the model. Unknown slash commands should print a short error
and suggest `/help`.

Defer these commands:

```text
/model <id>
/cwd <path>
/index
/rag eval <query>
```

They are useful, but each requires runtime rebuild or additional command parsing. Add them after the
core REPL loop is stable.

### Cancellation And Signals

One-shot can keep its current signal behavior: Ctrl+C cancels the run and exits with the existing
signal-derived code.

REPL should differ:

- Ctrl+C at prompt: clear current input and continue.
- Ctrl+C while a run is active: cancel only that run, record it as cancelled, and return to prompt.
- Ctrl+D at prompt: exit 0.
- SIGTERM should exit the process.

In implementation terms, each REPL run should create a child `CancellationToken`. The signal handler
for an active run cancels that token without poisoning the whole REPL loop.

### Rendering

Move the event rendering currently in `oneshot.rs` into a shared renderer. The renderer should:

- print `LlmChunk` deltas to stdout;
- print tool starts/results/errors to stderr as today;
- print plan and step events as today;
- record events through `RunArtifactRecorder`;
- finalize report artifacts after completion;
- return `TerminationReason`.

`oneshot.rs` and `repl.rs` should call the same renderer so behavior stays consistent.

The renderer may take a small options struct:

```rust
pub struct CliRunRenderOptions {
    pub print_done_line: bool,
    pub print_trailing_newline: bool,
}
```

One-shot can preserve current output. REPL can tune output slightly if needed, but should not fork
rendering logic.

### Error Handling

- Runtime build failure before entering REPL should return an error.
- Per-run model/tool errors should complete that run, record artifacts, and return to the prompt if
  possible.
- Slash command failures should print a short message and continue.
- History file failures should warn and continue.
- Resume failures should print the error and keep the previous `active_resume_state`.

### Tests

Add focused tests rather than trying to test terminal behavior end to end only.

Suggested Rust tests:

- `Args::parse_from(["rove"])` has no message and no command, and is not a help fast path.
- `rove "<task>"` still parses as one-shot message.
- subcommands still parse without message.
- slash command parser recognizes `/help`, `/exit`, `/resume latest`, `/resume <run_id>`, unknown.
- `/sessions` formatting reuses existing formatter.
- renderer tests with fake streams ensure tool/model events print and record expected terminal reason.
- REPL state helper chooses the correct resume state after a completed run.

Suggested integration smoke:

- Running the binary with no args under a controlled stdin containing `/exit` exits 0.
- Running the binary with a fake model and a one-shot message exits after one run and does not wait
  for REPL input.

## Phase 2: Remote RAG Rerank

### Goals

- Make existing rerank config fields executable.
- Keep deterministic/noop behavior as the default.
- Add a clean rerank interface rather than embedding provider-specific HTTP calls inside retrieval.
- Reuse existing model health/fallback patterns where practical.
- Record reranker identity in eval reports.
- Keep all RAG code behind the existing `rag` feature gate.

### Non-Goals

- Do not build a full ragent-style provider registry in this pass.
- Do not introduce Milvus, Redis, DB-backed KB tables, or enterprise intent trees.
- Do not change Browser/Desktop scope.
- Do not require remote rerank for tests.
- Do not remove deterministic local RAG mode.
- Do not make default builds depend on RAG-only dependencies.

### RAGENT Pattern To Adapt

Adapt these ideas from RAGENT:

```text
retrieval channels
  -> postprocessor chain
    -> rerank service
      -> provider client
      -> noop fallback
```

Do not copy the Java/Spring wiring. In `rove`, this should be a small Rust trait and client layer
under `src/tools/rag/`.

### Module Layout

Add:

```text
src/tools/rag/rerank.rs
```

Recommended contents:

```rust
#[async_trait]
pub trait Reranker: Send + Sync {
    async fn rerank(
        &self,
        query: &str,
        candidates: Vec<RetrievedChunk>,
        top_n: usize,
    ) -> anyhow::Result<Vec<RetrievedChunk>>;

    fn client_id(&self) -> ModelClientId;
}

pub struct NoopReranker;
pub struct OpenAiCompatibleReranker;
pub struct RoutingReranker;
```

The exact provider name can be adjusted based on supported API shape. If the first real provider is
DashScope/Bailian, name the concrete client accordingly and keep the trait provider-neutral.

Update:

```text
src/tools/rag/mod.rs
src/tools/rag/retrieve/pipeline.rs
src/tools/rag/retrieve/postprocess.rs
src/tools/rag/eval.rs
src/interfaces/cli/index.rs
```

### Interface Shape

`RetrievalPipeline` should accept both embedder and reranker:

```rust
pub struct RetrievalPipeline<'a> {
    index: &'a RagIndex,
    embedder: &'a dyn Embedder,
    reranker: &'a dyn Reranker,
}
```

For backwards compatibility during refactor, implementation can add:

```rust
RetrievalPipeline::new(index, embedder)
RetrievalPipeline::with_reranker(index, embedder, reranker)
```

where `new()` uses `NoopReranker`.

The postprocessor chain should replace `NoopRerankPostProcessor` with a reranker-backed processor.
Because rerank is async, either:

- make `SearchResultPostProcessor::process` async, or
- keep dedupe/normalization as synchronous postprocessors and call async rerank explicitly at the end
  of `RetrievalPipeline::run`.

Recommendation: call async rerank explicitly at the end of `RetrievalPipeline::run` for the first
pass. This keeps the synchronous postprocessor trait small and avoids a broad async trait migration.

Flow:

```text
channels run
merge channel results
dedupe
score normalize and preliminary truncate/cap
reranker.rerank(normalized_query, candidates, limit)
return results
```

The candidate count passed to rerank should be higher than the final limit when possible. If current
channels already return only `limit`, add a small multiplier later. First pass can rerank the current
candidate list and still improve architecture.

### Provider Client

Support one remote rerank API shape initially. Prefer the shape that matches the configured provider
you expect to use first.

For DashScope/Bailian-style rerank, request shape:

```json
{
  "model": "qwen3-rerank",
  "input": {
    "query": "where is rerank wired",
    "documents": [
      "Remote rerank configuration exists in AppConfig.",
      "NoopRerankPostProcessor currently truncates candidates."
    ]
  },
  "parameters": {
    "top_n": 5,
    "return_documents": true
  }
}
```

Response parsing should map returned `index` values back onto the original `RetrievedChunk` values,
replace score when a `relevance_score` exists, preserve path/source/heading/content, and fill any
missing slots with original candidates in their current order.

If implementing an OpenAI-compatible rerank endpoint instead, keep the same internal trait and only
change the concrete HTTP request/response parser.

### Config

Existing fields:

```text
rag.rerank_provider
rag.rerank_model
rag.rerank_api_key
rag.timeout_ms
rag.fallback_to_deterministic
```

Add only if needed:

```text
rag.rerank_api_base
```

Current config has `embedding_api_base` but no rerank-specific base. If rerank provider can share the
embedding/model base, document that explicitly. If not, add `rag.rerank_api_base` with a safe default.

Recommended behavior:

- If `rag.rerank_provider` or `rag.rerank_model` is unset, use `NoopReranker`.
- If remote rerank is configured but API key is missing:
  - if `fallback_to_deterministic = true`, warn and use `NoopReranker`;
  - otherwise return a config error.
- If remote rerank call fails:
  - if fallback is enabled, mark failure and return original candidates truncated to `top_n`;
  - otherwise return the error.

### Health And Fallback

Reuse `ModelHealthStore` semantics for remote rerank where practical:

- `RoutingReranker` should try candidates in order.
- For the first pass, candidates can be simple: remote first, noop second.
- Remote failures should call `mark_failure`.
- Remote success should call `mark_success`.
- Open circuits should skip remote and use fallback.

Do not block the first pass on a general model selector if that makes the scope too large. The key is
to keep provider calls behind `Reranker` and avoid hardcoding remote HTTP inside retrieval.

### Eval Reports

Change eval reporting from:

```text
reranker: "none"
```

to the actual `Reranker::client_id()`.

Examples:

```text
rerank-noop
rerank-routing
rerank-dashscope:https://dashscope.aliyuncs.com:qwen3-rerank
```

### Tool Usage

`RagRetrieveTool` currently constructs `DeterministicEmbedder` directly. Phase 2 will keep agent
tool-time RAG retrieval deterministic unless the implementation can pass config into RAG tools with
a small local change.

- Indexing/eval use configurable embedder/reranker.
- Agent tool retrieval can remain deterministic unless runtime tool construction is extended to pass
  `AppConfig` into RAG tools.

Better follow-up:

- Extend `runtime_tool_registry()` to accept resolved RAG provider config or a RAG service bundle.
- Build RAG tools with configured embedder/reranker.

Do not mix this larger registry change into the minimum remote rerank pass unless implementation
shows it is small.

### Tests

Unit tests:

- `NoopReranker` truncates and preserves order.
- remote rerank maps returned indexes to original chunks.
- remote rerank preserves original chunks for missing/invalid returned indexes.
- missing API key falls back or errors according to config.
- `RoutingReranker` falls back after remote failure.
- health store skips open remote target.

RAG tests:

- retrieval pipeline with a fake reranker returns reranked order.
- eval report records fake reranker identity.
- default pipeline still reports noop and preserves existing result behavior.

CLI/config tests:

- `dump-config` redacts rerank key presence.
- env vars `ROVE_RAG_RERANK_PROVIDER`, `ROVE_RAG_RERANK_MODEL`, `ROVE_RAG_RERANK_API_KEY` still map correctly.

Verification commands after implementation:

```powershell
cargo fmt --all --check
cargo test
cargo test --features rag
cargo clippy --all-targets --features rag -- -D warnings
```

If the REPL adds terminal integration tests that are environment-sensitive, keep them focused and
avoid brittle timing.

## Implementation Sequencing

Recommended sequence for a future plan:

1. Add REPL dependency and update arg fast-path behavior.
2. Extract shared CLI runtime builder from `src/main.rs`.
3. Extract shared run renderer from `oneshot.rs`.
4. Add `repl.rs` with slash command parsing and a testable state helper.
5. Wire no-message/no-command startup to REPL.
6. Add REPL tests and one-shot regression tests.
7. Add `Reranker` trait and `NoopReranker`.
8. Wire retrieval/eval to accept reranker identity without changing default behavior.
9. Add remote rerank client and config builder.
10. Add routing/fallback/health around rerank.
11. Add RAG tests and docs update for implementation status.

## Risks And Mitigations

| Risk | Mitigation |
|---|---|
| REPL refactor changes one-shot behavior | Keep one-shot regression tests and share renderer instead of rewriting output twice |
| Ctrl+C handling becomes inconsistent | Use separate cancellation tokens for app lifetime and active REPL run |
| REPL state identity gets confusing | Start with a single REPL session and explicit resume state; document job-id behavior in tests |
| Rerank API shapes differ by provider | Keep provider-specific parsing isolated behind `Reranker` |
| Remote rerank makes local tests flaky | Default to noop and test remote behavior with fake HTTP/client mocks |
| RAG tool retrieval ignores provider config | Treat configured tool-time RAG as a follow-up unless the registry change is small |

## Acceptance Criteria

Phase 1 is done when:

- `rove` starts an interactive `rove> ` prompt.
- `rove "task"` still runs once and exits.
- existing subcommands still exit after completion.
- `/help`, `/exit`, `/quit`, `/clear`, `/sessions`, and `/resume` work.
- prompt history persists under state dir.
- Ctrl+D exits cleanly.
- Ctrl+C at prompt continues; Ctrl+C during a run cancels that run without exiting the REPL.
- CLI tests and existing default tests pass.

Phase 2 is done when:

- default RAG behavior remains deterministic/noop.
- configured remote rerank can reorder retrieved chunks.
- rerank failures degrade to noop when fallback is enabled.
- eval reports include actual reranker identity.
- RAG tests pass with `--features rag`.
- docs/runtime implementation guide no longer says remote rerank execution is unwired.

## Open Follow-Ups

These are intentionally out of scope for this implementation batch:

- Full-screen terminal UI with `ratatui`.
- Dynamic REPL runtime mutation via `/model` and `/cwd`.
- REPL multiline input.
- Browser/Desktop workspaces.
- General provider registry for all embedding/rerank/chat candidates.
- RAG tool-time provider config if it requires large registry changes.
