# CLI REPL And RAG Rerank Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the CLI REPL and remote RAG rerank design in `docs/superpowers/specs/2026-05-29-cli-repl-and-rag-rerank-design.md`.

**Architecture:** Keep `main.rs` as routing only, move shared CLI runtime construction into `src/interfaces/cli/runtime.rs`, and move run stream rendering/artifact recording into `src/interfaces/cli/render.rs`. Add a line-oriented `rustyline` REPL in `src/interfaces/cli/repl.rs`. Add a feature-gated RAG reranker trait/client layer in `src/tools/rag/rerank.rs` and call it explicitly at the end of retrieval.

**Tech Stack:** Rust 2024, Tokio, clap, rustyline, reqwest, existing `ModelHealthStore`, SQLite-backed `StateStore`, feature-gated `rag` modules.

---

## File Structure

- Modify `Cargo.toml` and `Cargo.lock`: add `rustyline`.
- Modify `src/main.rs`: parse args and route to dump-config, subcommands, one-shot, or REPL.
- Modify `src/interfaces/cli/args.rs`: keep no-arg parse valid and make only `dump-config` a sync fast path.
- Create `src/interfaces/cli/runtime.rs`: build workspace, config, model, registry, context manager, engine, and state store once for CLI callers.
- Create `src/interfaces/cli/render.rs`: shared stream renderer/recorder returning `TerminationReason`.
- Create `src/interfaces/cli/repl.rs`: slash command parser, REPL state helper, history handling, signal-aware run loop.
- Modify `src/interfaces/cli/oneshot.rs`: delegate stream work to `render.rs`.
- Modify `src/interfaces/cli/mod.rs`: export new modules.
- Create `tests/cli_repl.rs`: smoke tests for `/exit` and no-REPL one-shot behavior.
- Create `src/tools/rag/rerank.rs`: `Reranker`, `NoopReranker`, DashScope-compatible remote client, and `RoutingReranker`.
- Modify `src/tools/rag/mod.rs`: export reranker types.
- Modify `src/tools/rag/retrieve/pipeline.rs`: accept a reranker and call it after sync postprocessors.
- Modify `src/tools/rag/retrieve/postprocess.rs`: remove `NoopRerankPostProcessor`; keep dedupe and score normalization sync.
- Modify `src/tools/rag/eval.rs`: accept reranker and report `reranker.client_id()`.
- Modify `src/interfaces/cli/index.rs`: build configured reranker for RAG eval.
- Modify `docs/runtime/implementation-guide.md`, `docs/runtime/implementation-status.md`, and `docs/runtime/subsystems.md`: remove “remote rerank unwired” language.

## Task 1: REPL Dependency And Arg Routing Tests

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `src/interfaces/cli/args.rs`

- [x] **Step 1: Add failing arg tests**

Add tests inside `src/interfaces/cli/args.rs`:

```rust
#[test]
fn no_args_enters_async_cli_path() {
    let args = Args::parse_from(["rove"]);

    assert!(args.message.is_none());
    assert!(args.command.is_none());
    assert!(!args.is_sync_fast_path());
}

#[test]
fn quoted_task_still_parses_as_one_shot_message() {
    let args = Args::parse_from(["rove", "analyze this project"]);

    assert_eq!(args.message.as_deref(), Some("analyze this project"));
    assert!(args.command.is_none());
}
```

- [x] **Step 2: Run failing tests**

Run: `cargo test args::tests::no_args_enters_async_cli_path args::tests::quoted_task_still_parses_as_one_shot_message`

Expected before implementation: `no_args_enters_async_cli_path` fails because no-message/no-command is still a sync help fast path.

- [x] **Step 3: Add rustyline and update fast-path behavior**

Run: `cargo add rustyline`

Change `Args::is_sync_fast_path()` to:

```rust
pub fn is_sync_fast_path(&self) -> bool {
    matches!(self.command, Some(Command::DumpConfig))
}
```

- [x] **Step 4: Verify tests pass**

Run: `cargo test args::tests::no_args_enters_async_cli_path args::tests::quoted_task_still_parses_as_one_shot_message`

Expected: both tests pass.

## Task 2: Shared CLI Runtime Builder

**Files:**
- Create: `src/interfaces/cli/runtime.rs`
- Modify: `src/interfaces/cli/mod.rs`
- Modify: `src/main.rs`

- [x] **Step 1: Write failing runtime construction test**

Add unit tests in `src/interfaces/cli/runtime.rs` using a temp workspace:

```rust
#[tokio::test]
async fn runtime_builder_rebases_configured_state_dir_into_workspace() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join(".env"),
        "",
    ).unwrap();

    let runtime = build_cli_runtime(CliRuntimeOptions {
        cwd: Some(tmp.path().to_path_buf()),
        model: Some("fake".to_string()),
        max_steps: Some(2),
        approval: CliApprovalPolicy::Never,
        task_workspace: None,
        task_base: None,
        initial_fake_response: Some("ready".to_string()),
    })
    .await
    .unwrap();

    assert_eq!(runtime.workspace.root, tmp.path().canonicalize().unwrap());
    assert!(runtime.workspace.state_dir.ends_with(".rove"));
    assert_eq!(runtime.config.provider.model, "fake");
}
```

- [x] **Step 2: Run failing test**

Run: `cargo test interfaces::cli::runtime::tests::runtime_builder_rebases_configured_state_dir_into_workspace`

Expected before implementation: module/function not found.

- [x] **Step 3: Implement runtime builder**

Create `CliRuntimeOptions` and `CliRuntime`:

```rust
pub struct CliRuntimeOptions {
    pub cwd: Option<PathBuf>,
    pub model: Option<String>,
    pub max_steps: Option<u32>,
    pub approval: CliApprovalPolicy,
    pub task_workspace: Option<String>,
    pub task_base: Option<PathBuf>,
    pub initial_fake_response: Option<String>,
}

pub struct CliRuntime {
    pub workspace: Workspace,
    pub config: AppConfig,
    pub engine: Engine,
    pub state_store: StateStore,
}
```

Move the runtime setup from `main.rs` into `build_cli_runtime(options).await`, preserving workspace detection, task workspace handling, config rebasing, tool registry construction, context budget, planner prompt, memory paths, compaction, stdin input provider, and approval provider.

- [x] **Step 4: Verify test passes**

Run: `cargo test interfaces::cli::runtime::tests::runtime_builder_rebases_configured_state_dir_into_workspace`

Expected: pass.

## Task 3: Shared CLI Run Renderer

**Files:**
- Create: `src/interfaces/cli/render.rs`
- Modify: `src/interfaces/cli/oneshot.rs`
- Modify: `src/interfaces/cli/mod.rs`

- [x] **Step 1: Write failing renderer unit test**

Add a test in `render.rs` using a fake stream:

```rust
#[tokio::test]
async fn render_events_returns_terminal_reason() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let state_store = StateStore::new(&workspace.state_dir);
    let run = state_store
        .start_run(SessionId::new(), JobId::new(), RunId::new())
        .unwrap();
    let events = futures::stream::iter(vec![
        StreamEvent::RunStarted {
            run_id: run.run_id,
            job_id: run.job_id,
            user_message: "hello".to_string(),
        },
        StreamEvent::LlmChunk {
            delta: "hi".to_string(),
        },
        StreamEvent::RunCompleted {
            reason: TerminationReason::Final,
            output: Some("hi".to_string()),
        },
    ]);

    let reason = render_run_events(
        events,
        CliRunRenderContext {
            message: "hello".to_string(),
            run,
            resume_state: None,
            state_store: &state_store,
            workspace: &workspace,
            model_id: "fake",
        },
        CliRunRenderOptions::default(),
    )
    .await;

    assert_eq!(reason, TerminationReason::Final);
}
```

- [x] **Step 2: Run failing test**

Run: `cargo test interfaces::cli::render::tests::render_events_returns_terminal_reason`

Expected before implementation: module/function not found.

- [x] **Step 3: Implement renderer and adapt one-shot**

Implement:

```rust
#[derive(Debug, Clone, Copy)]
pub struct CliRunRenderOptions {
    pub print_done_line: bool,
    pub print_trailing_newline: bool,
}
```

Use it from `run_oneshot_with_cancel` by creating the engine stream with `engine.run_with_cancel(...)` and passing it to `render_run_events`. Preserve current stdout/stderr event rendering, artifact recording, and report finalization.

- [x] **Step 4: Verify renderer and one-shot tests**

Run: `cargo test interfaces::cli::render::tests::render_events_returns_terminal_reason`

Expected: pass.

## Task 4: REPL Slash Commands And State Helper

**Files:**
- Create: `src/interfaces/cli/repl.rs`
- Modify: `src/interfaces/cli/mod.rs`

- [x] **Step 1: Write failing slash command tests**

Add tests in `repl.rs`:

```rust
#[test]
fn slash_command_parser_recognizes_first_pass_commands() {
    assert_eq!(SlashCommand::parse("/help"), SlashCommand::Help);
    assert_eq!(SlashCommand::parse("/exit"), SlashCommand::Exit);
    assert_eq!(SlashCommand::parse("/quit"), SlashCommand::Exit);
    assert_eq!(SlashCommand::parse("/clear"), SlashCommand::Clear);
    assert_eq!(SlashCommand::parse("/sessions"), SlashCommand::Sessions);
    assert_eq!(SlashCommand::parse("/resume latest"), SlashCommand::ResumeLatest);
    assert_eq!(
        SlashCommand::parse("/resume 01ARYZ6S41YYYYYYYYYYYYYYYY"),
        SlashCommand::ResumeRun("01ARYZ6S41YYYYYYYYYYYYYYYY".to_string())
    );
    assert_eq!(
        SlashCommand::parse("/model gpt"),
        SlashCommand::Unknown("/model".to_string())
    );
}

#[test]
fn repl_state_uses_previous_task_identity_for_follow_up() {
    let session_id = SessionId::new();
    let first = ReplState::new(session_id);
    let first_identity = first.next_run_identity();
    let completed = task_state(session_id, first_identity.job_id, first_identity.run_id);
    let resumed = first.with_active_resume_state(Some(completed.clone()));
    let next_identity = resumed.next_run_identity();

    assert_eq!(next_identity.session_id, session_id);
    assert_eq!(next_identity.job_id, completed.job_id);
    assert_ne!(next_identity.run_id, completed.run_id);
}
```

- [x] **Step 2: Run failing tests**

Run: `cargo test interfaces::cli::repl::tests::slash_command_parser_recognizes_first_pass_commands interfaces::cli::repl::tests::repl_state_uses_previous_task_identity_for_follow_up`

Expected before implementation: module/function not found.

- [x] **Step 3: Implement parser and state helper**

Implement:

```rust
pub enum SlashCommand {
    Help,
    Exit,
    Clear,
    Sessions,
    ResumeLatest,
    ResumeRun(String),
    Unknown(String),
}

pub struct ReplState {
    session_id: SessionId,
    active_resume_state: Option<TaskState>,
}

pub struct ReplRunIdentity {
    pub session_id: SessionId,
    pub job_id: JobId,
    pub run_id: RunId,
}
```

The helper must keep one `SessionId`, reuse the active resume state's `JobId` when present, and generate a new `RunId` for every prompt.

- [x] **Step 4: Verify tests pass**

Run: `cargo test interfaces::cli::repl::tests::slash_command_parser_recognizes_first_pass_commands interfaces::cli::repl::tests::repl_state_uses_previous_task_identity_for_follow_up`

Expected: pass.

## Task 5: REPL Loop, History, Startup Routing, And Smoke Tests

**Files:**
- Modify: `src/interfaces/cli/repl.rs`
- Modify: `src/main.rs`
- Create: `tests/cli_repl.rs`

- [x] **Step 1: Write failing integration smoke tests**

Create tests that run the compiled binary:

```rust
#[test]
fn no_args_accepts_exit_command_and_exits_zero() {
    let tmp = tempfile::TempDir::new().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rove"))
        .arg("--cwd")
        .arg(tmp.path())
        .arg("--model")
        .arg("fake")
        .arg("--approval")
        .arg("never")
        .env("ROVE_STATE_ALLOW_EXTERNAL_PATHS", "true")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write as _;
            child.stdin.as_mut().unwrap().write_all(b"/exit\n")?;
            child.wait_with_output()
        })
        .unwrap();

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("rove REPL - type /help for commands, /exit to quit"));
    assert!(tmp.path().join(".rove").join("repl_history").exists());
}

#[test]
fn one_shot_message_does_not_wait_for_repl_input() {
    let tmp = tempfile::TempDir::new().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rove"))
        .arg("--cwd")
        .arg(tmp.path())
        .arg("--model")
        .arg("fake")
        .arg("--approval")
        .arg("never")
        .arg("hello")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("fake response: hello"));
}
```

- [x] **Step 2: Run failing smoke tests**

Run: `cargo test --test cli_repl`

Expected before implementation: no-arg path prints help or smoke test does not see REPL behavior.

- [x] **Step 3: Implement REPL loop and main routing**

Use `rustyline::DefaultEditor`, load/save history at `<state_dir>/repl_history`, print the compact startup line, execute slash commands locally, and execute ordinary input via the shared runtime and renderer. `Ctrl+D` exits. `Ctrl+C` at the prompt continues. During a run, create a child `CancellationToken` and cancel only that run on Ctrl+C while SIGTERM exits the process.

Update `main.rs` route:

```rust
if message.is_some() {
    run_one_shot_from_runtime(...).await
} else {
    repl::run(runtime).await
}
```

- [x] **Step 4: Verify smoke tests pass**

Run: `cargo test --test cli_repl`

Expected: both tests pass.

## Task 6: Reranker Trait And Noop Behavior

**Files:**
- Create: `src/tools/rag/rerank.rs`
- Modify: `src/tools/rag/mod.rs`

- [x] **Step 1: Write failing RAG unit tests**

Add tests in `rerank.rs`:

```rust
#[tokio::test]
async fn noop_reranker_truncates_and_preserves_order() {
    let chunks = vec![chunk("a", 0.9), chunk("b", 0.8), chunk("c", 0.7)];
    let reranked = NoopReranker
        .rerank("query", chunks.clone(), 2)
        .await
        .unwrap();

    assert_eq!(reranked.len(), 2);
    assert_eq!(reranked[0].id, "a");
    assert_eq!(reranked[1].id, "b");
    assert_eq!(NoopReranker.client_id().to_string(), "rerank-noop");
}
```

- [x] **Step 2: Run failing test**

Run: `cargo test --features rag tools::rag::rerank::tests::noop_reranker_truncates_and_preserves_order`

Expected before implementation: module/type not found.

- [x] **Step 3: Implement trait and noop**

Implement:

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
```

`NoopReranker` truncates to `top_n` and returns `ModelClientId::opaque("rerank-noop")`.

- [x] **Step 4: Verify test passes**

Run: `cargo test --features rag tools::rag::rerank::tests::noop_reranker_truncates_and_preserves_order`

Expected: pass.

## Task 7: Retrieval Pipeline And Eval Reranker Identity

**Files:**
- Modify: `src/tools/rag/retrieve/pipeline.rs`
- Modify: `src/tools/rag/retrieve/postprocess.rs`
- Modify: `src/tools/rag/eval.rs`
- Modify: `tests/cli_index.rs`

- [x] **Step 1: Write failing pipeline/eval tests**

Add a fake reranker test to `pipeline.rs` and update CLI index eval expectations:

```rust
struct ReverseReranker;

#[async_trait]
impl Reranker for ReverseReranker {
    async fn rerank(
        &self,
        _query: &str,
        mut candidates: Vec<RetrievedChunk>,
        top_n: usize,
    ) -> anyhow::Result<Vec<RetrievedChunk>> {
        candidates.reverse();
        candidates.truncate(top_n);
        Ok(candidates)
    }

    fn client_id(&self) -> ModelClientId {
        ModelClientId::opaque("rerank-reverse")
    }
}
```

Assert `RetrievalPipeline::with_reranker(...).run(...).results[0]` reflects the reranked order and `run_retrieval_eval_with_reranker(...).reranker == "rerank-reverse"`.

- [x] **Step 2: Run failing tests**

Run: `cargo test --features rag retrieval_pipeline_uses_configured_reranker eval_report_records_reranker_identity`

Expected before implementation: functions/types not found or report still says `none`.

- [x] **Step 3: Wire reranker explicitly**

Add `RetrievalPipeline::with_reranker(index, embedder, reranker)` and keep `RetrievalPipeline::new(index, embedder)` using `NoopReranker`. Keep dedupe and score normalization as synchronous postprocessors, then call:

```rust
let results = self
    .reranker
    .rerank(&context.normalized_query, results, limit)
    .await?;
```

Add `run_retrieval_eval_with_reranker` and make existing `run_retrieval_eval` call it with `NoopReranker`.

- [x] **Step 4: Verify tests pass**

Run: `cargo test --features rag retrieval_pipeline_uses_configured_reranker eval_report_records_reranker_identity`

Expected: pass.

## Task 8: Remote Rerank Client, Config Builder, Routing, And Health

**Files:**
- Modify: `src/tools/rag/rerank.rs`
- Modify: `src/interfaces/cli/index.rs`

- [x] **Step 1: Write failing rerank tests**

Add tests for response mapping, invalid indexes, missing key behavior, fallback after failure, and health skip:

```rust
#[test]
fn build_rag_reranker_requires_key_when_fallback_disabled() {
    let mut config = AppConfig::default();
    config.rag.rerank_provider = Some("dashscope".to_string());
    config.rag.rerank_model = Some("qwen3-rerank".to_string());
    config.rag.rerank_api_key = None;
    config.rag.fallback_to_deterministic = false;

    let err = build_rag_reranker(&config).unwrap_err();

    assert!(err.to_string().contains("rag.rerank_api_key is required"));
}
```

Use a local `tokio::net::TcpListener`/`axum::Router` test server for remote JSON mapping and failure fallback.

- [x] **Step 2: Run failing tests**

Run: `cargo test --features rag rerank_`

Expected before implementation: tests fail because only noop exists.

- [x] **Step 3: Implement remote client and routing**

Implement a DashScope-compatible request to `{api_base}/services/rerank/text-rerank/text-rerank`, parse returned `output.results[].index` and `relevance_score`, map indexes back to original `RetrievedChunk` values, and append missing originals in original order. Implement `RoutingReranker` using `ModelHealthStore` exactly like `RoutingEmbedder`, with remote first and noop fallback.

Expose `build_rag_reranker(config: &AppConfig) -> anyhow::Result<Box<dyn Reranker>>` under the `rag` feature. Use `config.rag.embedding_api_base` as the rerank base for this pass and document that choice in code/docs.

- [x] **Step 4: Verify rerank tests pass**

Run: `cargo test --features rag rerank_`

Expected: pass.

## Task 9: CLI RAG Eval Uses Configured Reranker

**Files:**
- Modify: `src/interfaces/cli/index.rs`
- Modify: `tests/cli_index.rs`

- [x] **Step 1: Write failing CLI/config tests**

Update CLI index eval tests so default eval reports `rerank-noop` instead of `none`, and add a focused test for `build_rag_reranker` fallback behavior.

- [x] **Step 2: Run failing tests**

Run: `cargo test --features rag --test cli_index`

Expected before implementation: eval report still records old value or builder unavailable.

- [x] **Step 3: Wire builder into eval path**

In `run_impl`, build `let reranker = build_rag_reranker(&config)?;` and call:

```rust
run_retrieval_eval_with_reranker(
    &index,
    embedder.as_ref(),
    reranker.as_ref(),
    kind,
    &query,
    options.eval_limit,
)
.await?
```

Indexing remains embedder-only; tool-time RAG remains deterministic.

- [x] **Step 4: Verify CLI RAG tests pass**

Run: `cargo test --features rag --test cli_index`

Expected: pass.

## Task 10: Documentation Status Updates

**Files:**
- Modify: `docs/runtime/implementation-guide.md`
- Modify: `docs/runtime/implementation-status.md`
- Modify: `docs/runtime/subsystems.md`

- [x] **Step 1: Search for stale language**

Run: `rg -n "remote rerank|unwired|NoopRerank|reranker = \"none\"|not wired" docs/runtime`

Expected before docs update: stale statements exist.

- [x] **Step 2: Update docs**

Replace stale language with:

```markdown
Remote rerank is optional. When `rag.rerank_provider` and `rag.rerank_model` are set, eval retrieval builds a routed reranker using the configured API key and the RAG embedding API base as the first-pass rerank endpoint base. If rerank is unconfigured or fallback is enabled after a provider failure, retrieval uses `rerank-noop` and keeps deterministic local behavior.
```

- [x] **Step 3: Verify stale language is gone**

Run: `rg -n "remote rerank execution is not wired|unwired|reranker = \"none\"" docs/runtime`

Expected: no matches.

## Task 11: Full Verification, Commit, Push, CI

**Files:**
- All changed files.

- [x] **Step 1: Format check**

Run: `cargo fmt --all --check`

Expected: exit 0.

- [x] **Step 2: Default tests**

Run: `cargo test`

Expected: exit 0.

- [x] **Step 3: RAG tests**

Run: `cargo test --features rag`

Expected: exit 0.

- [x] **Step 4: Clippy**

Run: `cargo clippy --all-targets --features rag -- -D warnings`

Expected: exit 0.

- [x] **Step 5: Requirement grep**

Run: `rg -n "remote rerank execution is not wired|NoopRerankPostProcessor|reranker = \"none\"" src docs/runtime tests`

Expected: no stale implementation/status matches.

- [ ] **Step 6: Commit**

Run:

```powershell
git add Cargo.toml Cargo.lock src tests docs
git commit -m "feat: add cli repl and rag rerank"
```

Expected: commit succeeds.

- [ ] **Step 7: Push**

Run: `git push -u origin feat/cli-repl-rag-rerank`

Expected: push succeeds.

- [ ] **Step 8: Check CI**

Run: `gh run list --branch feat/cli-repl-rag-rerank --limit 5`

Expected: most recent workflow for the pushed branch completes successfully. If no workflow appears immediately, wait and re-check.
