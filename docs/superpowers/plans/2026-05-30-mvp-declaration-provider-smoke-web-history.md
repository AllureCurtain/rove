# MVP Declaration, Provider Smoke, and Web History Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Mark the current rove runtime as an achieved local-first MVP, document the MVP boundary, add real-provider opt-in smoke coverage, and let the Web workbench inspect historical runs and reports.

**Architecture:** Keep `docs/runtime/` as the source of truth. Add small API read endpoints over existing SQLite/artifact state instead of changing the engine. Extend the Web workbench through typed client functions and reducer state, preserving the current single-page console model.

**Tech Stack:** Rust 2024, axum, rusqlite, serde, tokio, Next.js 16, React 19, TypeScript, Vitest, Playwright optional, Cargo tests.

---

## Scope

This plan implements the first four recommendations from the MVP assessment:

1. Formally mark the current runtime as **MVP reached**.
2. Add an explicit MVP definition document that says what is included and excluded.
3. Add opt-in real provider smoke verification for OpenAI-compatible, Anthropic, and Ollama paths.
4. Add Web workbench history/report viewing for persisted runs.

This plan does not implement Browser/Desktop workspaces, provider-backed tool-time RAG, multi-user identity, distributed rate limiting, or shell sandboxing.

---

## Problem Statement

The project has enough runtime surface to qualify as a local-first MVP: CLI, API, Web, streaming events, tools, state artifacts, resume, memory, MCP, and deterministic benchmarks already exist. The remaining issue is not core capability. The issue is product/readiness framing and evidence:

- The docs still read like a hardening roadmap rather than a clear MVP declaration.
- New readers do not have one stable document that defines the MVP boundary.
- The automated suite proves fake/deterministic behavior well, but real provider paths are not captured as a repeatable smoke checklist.
- The Web UI can run and resume jobs, but it cannot browse old runs or inspect generated `report.json` artifacts.

The implementation should improve confidence and product usability without broadening the runtime vision.

---

## File Structure

### Documentation

- Create `docs/runtime/mvp-definition.md`
  - Defines MVP reached status, included capabilities, excluded capabilities, golden paths, and acceptance baseline.
- Modify `docs/runtime/README.md`
  - Adds the MVP definition to the runtime docs index.
- Modify `docs/runtime/implementation-status.md`
  - Adds a top-level MVP status summary and links to `mvp-definition.md`.
- Modify `docs/runtime/implementation-guide.md`
  - Adds a short provider smoke section and Web history/report viewing notes.
- Modify `README.md`
  - Adds a concise “Current MVP” section.
- Modify `docs/runtime/implementation-status.md`
  - Adds an explicit top-level MVP status summary.
- Modify `tests/code_hygiene.rs`
  - Adds doc coverage assertions so the MVP docs do not drift silently.

### Provider Smoke

- Create `docs/runtime/provider-smoke.md`
  - Operator-facing smoke procedure for OpenAI-compatible, Anthropic, and Ollama.
- Create `tests/provider_smoke.rs`
  - Env-gated integration tests that run only when explicit provider smoke env vars are present.
- Modify `Cargo.toml`
  - No new dependencies expected. Only add test target if the repo style requires explicit test target entries; otherwise leave untouched.

### API History/Reports

- Modify `src/state/index.rs`
  - Add read methods for recent run records and report records.
- Modify `src/state/store.rs`
  - Add a safe report loader by run id using indexed report path.
- Modify `src/interfaces/api/mod.rs`
  - Add `GET /runs` and `GET /runs/{run_id}/report`.
- Modify `tests/api.rs`
  - Add API tests for listing historical runs and loading a report after completion/restart.

### Web History/Reports

- Modify `web-ui/lib/rove-types.ts`
  - Add `RunSummary`, `RunReport`, and response types.
- Modify `web-ui/lib/rove-client.ts`
  - Add `listRuns` and `fetchRunReport`.
- Modify `web-ui/lib/rove-client.test.ts`
  - Add endpoint tests.
- Modify `web-ui/lib/rove-state.ts`
  - No required change for the first implementation; keep selected report in component-local state.
- Modify `web-ui/components/rove-workbench.tsx`
  - Add a history panel and report detail view.
- Modify `web-ui/lib/rove-state.test.ts`
  - No required change unless implementation later moves report state into the reducer.
- Modify `web-ui/tests/e2e/workbench.spec.ts`
  - Add browser coverage for history/report loading only if the existing Playwright setup already mocks `/api/runs`; otherwise keep browser coverage outside the first task.

---

## Work Package 1: Declare MVP Reached

### Problem

The runtime docs say the M0-M6 acceptance matrix is met, but they do not state the product conclusion plainly. This makes the project look like it is still pre-MVP even though the local-first CLI/API/Web path is already present.

### Implementation

Add a short, dated MVP declaration to runtime docs and root README. The declaration should say:

- MVP reached for local-first single-user runtime.
- Current proof: `cargo test`, Web tests, deterministic benchmarks, acceptance matrix.
- MVP excludes Browser/Desktop workspaces, SaaS multi-user deployment, deep RAG runtime integration, and hardened shell sandboxing.

### Acceptance Standard

- A new reader can find the MVP status from `README.md` in one click.
- `docs/runtime/implementation-status.md` has an explicit “MVP Status” section.
- Existing `docs/runtime/acceptance-matrix.md` remains the proof map, not duplicated.
- `cargo test --test code_hygiene` verifies the docs mention the MVP declaration and source-of-truth path.

### Task 1: Add MVP Definition Document

**Files:**
- Create: `docs/runtime/mvp-definition.md`
- Modify: `docs/runtime/README.md`
- Modify: `README.md`
- Modify: `docs/runtime/implementation-status.md`
- Modify: `tests/code_hygiene.rs`

- [ ] **Step 1: Write the failing doc coverage test**

Add this test to `tests/code_hygiene.rs`:

```rust
#[test]
fn runtime_docs_declare_current_mvp_boundary() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let runtime_readme = std::fs::read_to_string(root.join("docs/runtime/README.md")).unwrap();
    let mvp_definition =
        std::fs::read_to_string(root.join("docs/runtime/mvp-definition.md")).unwrap();
    let implementation_status =
        std::fs::read_to_string(root.join("docs/runtime/implementation-status.md")).unwrap();
    let root_readme = std::fs::read_to_string(root.join("README.md")).unwrap();

    assert!(
        runtime_readme.contains("mvp-definition.md"),
        "runtime README should link to the MVP definition"
    );
    assert!(
        root_readme.contains("Current MVP"),
        "root README should expose the current MVP status"
    );
    assert!(
        implementation_status.contains("MVP Status"),
        "implementation status should expose the MVP status"
    );
    assert!(
        mvp_definition.contains("MVP reached"),
        "MVP definition should explicitly declare the reached state"
    );
    assert!(
        mvp_definition.contains("Out of scope"),
        "MVP definition should name exclusions"
    );
    assert!(
        mvp_definition.contains("Browser/Desktop"),
        "MVP definition should keep future workspace surfaces out of scope"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```powershell
cargo test --test code_hygiene runtime_docs_declare_current_mvp_boundary -- --exact
```

Expected: FAIL because `docs/runtime/mvp-definition.md` does not exist yet.

- [ ] **Step 3: Create `docs/runtime/mvp-definition.md`**

Create the file with this content:

```markdown
# MVP Definition

Status: MVP reached for the local-first single-user runtime.
Date: 2026-05-30

## Definition

The rove MVP is a local-first agent runtime that can run from CLI, API, and Web, stream observable events, call bounded tools, persist readable artifacts, resume from saved state, and run deterministic verification without network credentials.

This MVP is not a SaaS product, browser automation runtime, desktop automation runtime, or multi-user hosted service.

## Included

- CLI one-shot runs and line-oriented REPL.
- HTTP API job lifecycle with SSE, cancel, approval, input, resume, and persisted replay.
- Standalone Web workbench for submitting jobs, streaming events, approving tools, answering input requests, cancelling runs, and resuming latest state.
- Core engine with planned and unplanned loops sharing model turns, tool turns, context checkpoints, and history writeback.
- Local state under `.rove/` with trace, task state, report, and SQLite index.
- Folder, Repo, and Task workspaces.
- Built-in filesystem, shell, memory, request-input, MCP, and feature-gated RAG tools.
- Provider abstraction for OpenAI-compatible, Anthropic, Ollama, and fake providers.
- Deterministic no-network benchmarks and default test coverage.

## Out of scope

- Browser/Desktop workspace implementations.
- Multi-user identity, login, hosted billing, distributed rate limiting, and SaaS deployment controls.
- Full shell sandboxing beyond current local policy, timeout, output, denylist, and approval controls.
- Provider-backed tool-time RAG retrieval as a default runtime path.
- Long-running human-in-the-loop reconstruction after process restart.

## Golden paths

1. CLI smoke:

   ```powershell
   cargo run -- --model fake "echo hello from rove"
   ```

2. API and Web smoke:

   ```powershell
   cargo run --bin rove-api
   cd web-ui
   pnpm dev
   ```

3. Deterministic benchmark:

   ```powershell
   cargo run --bin rove-bench -- --suite benchmarks/agent-smoke.json --output-dir .rove/bench
   ```

4. Resume state:

   ```powershell
   cargo run -- --model fake "inspect this workspace"
   cargo run -- sessions
   cargo run -- --resume latest --model fake "continue"
   ```

## Required verification baseline

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
cd web-ui
pnpm test
pnpm typecheck
pnpm build
```

Optional RAG verification remains separate:

```powershell
cargo check --features rag --bin rove-index
cargo test --features rag
```
```

- [ ] **Step 4: Link the MVP definition from `docs/runtime/README.md`**

Add this row to the Documents table:

```markdown
| [mvp-definition.md](mvp-definition.md) | Current local-first MVP boundary, included capabilities, exclusions, golden paths, and verification baseline. |
```

- [ ] **Step 5: Add root README summary**

Add this section near the top of `README.md`, after the opening architecture paragraph:

```markdown
## Current MVP

rove has reached its local-first MVP: CLI, API, Web workbench, streaming events, bounded tool execution, persisted state, resume, deterministic benchmarks, and runtime docs are all present. The current MVP boundary is documented in [docs/runtime/mvp-definition.md](docs/runtime/mvp-definition.md).

Browser/Desktop workspaces, hosted multi-user identity, distributed rate limiting, and deeper provider-backed tool-time RAG are outside this MVP.
```

- [ ] **Step 6: Add implementation-status summary**

Add this section near the top of `docs/runtime/implementation-status.md`, before the current matrix:

```markdown
## MVP Status

MVP reached for the local-first single-user runtime. The exact boundary, included capabilities, exclusions, golden paths, and verification baseline are documented in [mvp-definition.md](mvp-definition.md).
```

- [ ] **Step 7: Run focused verification**

Run:

```powershell
cargo test --test code_hygiene runtime_docs_declare_current_mvp_boundary -- --exact
```

Expected: PASS.

- [ ] **Step 8: Commit**

```powershell
git add README.md docs/runtime/README.md docs/runtime/implementation-status.md docs/runtime/mvp-definition.md tests/code_hygiene.rs
git commit -m "docs: declare local-first mvp boundary"
```

---

## Work Package 2: Add Provider Smoke Verification

### Problem

The deterministic suite proves the engine and fake model path well, but real provider integration is currently verified mostly by unit-level adapter tests. There is no repeatable operator smoke for OpenAI-compatible, Anthropic, and Ollama end-to-end runs.

### Implementation

Add env-gated tests in `tests/provider_smoke.rs`. These tests must be skipped by default and only run when explicit opt-in env vars are present. Each smoke should use the public CLI/API/runtime path enough to catch request formatting and streaming normalization issues.

Recommended gates:

- `ROVE_PROVIDER_SMOKE_OPENAI=1`
- `ROVE_PROVIDER_SMOKE_ANTHROPIC=1`
- `ROVE_PROVIDER_SMOKE_OLLAMA=1`

Provider config should come from existing env variables where possible:

- OpenAI-compatible: `OPENAI_API_KEY`, `OPENAI_API_BASE`, `ROVE_PROVIDER_SMOKE_OPENAI_MODEL`
- Anthropic: `ANTHROPIC_API_KEY`, `ROVE_PROVIDER_SMOKE_ANTHROPIC_MODEL`
- Ollama: `ROVE_PROVIDER_SMOKE_OLLAMA_MODEL`; the smoke uses the current Ollama client default base URL unless existing config/env support already overrides it.

The smoke tasks should be intentionally tiny: one final-answer request and one tool-use request where the provider supports native tools.

### Acceptance Standard

- Default `cargo test` passes with smoke tests skipped.
- Setting a provider smoke env var runs a real provider test and fails clearly if credentials or model config are missing.
- `docs/runtime/provider-smoke.md` explains exact commands and env vars.
- `docs/runtime/implementation-guide.md` links to the provider smoke doc.

### Task 2: Add Provider Smoke Docs and Env-Gated Tests

**Files:**
- Create: `docs/runtime/provider-smoke.md`
- Create: `tests/provider_smoke.rs`
- Modify: `docs/runtime/implementation-guide.md`
- Modify: `docs/runtime/README.md`

- [ ] **Step 1: Create the provider smoke test file**

Create `tests/provider_smoke.rs` with this structure:

```rust
use futures::StreamExt;
use rove::config::{AppConfig, AppConfigOverrides};
use rove::core::context::ContextManager;
use rove::core::engine::{Engine, EngineConfig};
use rove::core::events::StreamEvent;
use rove::core::workspace::Workspace;
use rove::models::factory::build_model_client;
use rove::tools::default_tool_registry;

fn smoke_enabled(name: &str) -> bool {
    std::env::var(name).ok().as_deref() == Some("1")
}

fn require_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!("{name} must be set when the matching provider smoke gate is enabled")
    })
}

async fn run_provider_smoke(provider: &str, model: String, message: &str) -> String {
    let workspace = Workspace::detect(std::env::current_dir().unwrap().as_path()).unwrap();
    let mut config = AppConfig::load(
        &workspace.root,
        AppConfigOverrides {
            model: Some(model.clone()),
            max_steps: Some(3),
            api_bind_addr: None,
        },
    )
    .unwrap();
    config.provider.name = provider.to_string();
    config.provider.model = model;

    let model = build_model_client(&config, config.provider.model.clone());
    let engine = Engine::with_workspace(
        model,
        default_tool_registry(&workspace),
        ContextManager::new(config.load_system_prompt()),
        EngineConfig {
            max_steps: 3,
            plan_enabled: false,
        },
        workspace,
        rove::core::types::ApprovalPolicy::Never,
    );

    let mut stream = engine.ask(message.to_string(), None);
    let mut final_output = None;
    while let Some(event) = stream.next().await {
        if let StreamEvent::RunCompleted { output, .. } = event {
            final_output = output;
            break;
        }
    }
    final_output.unwrap_or_default()
}

#[tokio::test]
async fn openai_compatible_real_provider_smoke_when_enabled() {
    if !smoke_enabled("ROVE_PROVIDER_SMOKE_OPENAI") {
        return;
    }
    require_env("OPENAI_API_KEY");
    let model = std::env::var("ROVE_PROVIDER_SMOKE_OPENAI_MODEL")
        .unwrap_or_else(|_| "gpt-4.1-mini".to_string());
    let output = run_provider_smoke(
        "openai-compatible",
        model,
        "Reply with exactly: rove provider smoke ok",
    )
    .await;
    assert!(
        output.to_ascii_lowercase().contains("rove provider smoke ok"),
        "unexpected provider smoke output: {output}"
    );
}

#[tokio::test]
async fn anthropic_real_provider_smoke_when_enabled() {
    if !smoke_enabled("ROVE_PROVIDER_SMOKE_ANTHROPIC") {
        return;
    }
    require_env("ANTHROPIC_API_KEY");
    let model = std::env::var("ROVE_PROVIDER_SMOKE_ANTHROPIC_MODEL")
        .unwrap_or_else(|_| "claude-3-5-haiku-latest".to_string());
    let output = run_provider_smoke(
        "anthropic",
        model,
        "Reply with exactly: rove provider smoke ok",
    )
    .await;
    assert!(
        output.to_ascii_lowercase().contains("rove provider smoke ok"),
        "unexpected provider smoke output: {output}"
    );
}

#[tokio::test]
async fn ollama_real_provider_smoke_when_enabled() {
    if !smoke_enabled("ROVE_PROVIDER_SMOKE_OLLAMA") {
        return;
    }
    let model = std::env::var("ROVE_PROVIDER_SMOKE_OLLAMA_MODEL")
        .unwrap_or_else(|_| "llama3.2".to_string());
    let output = run_provider_smoke(
        "ollama",
        model,
        "Reply with exactly: rove provider smoke ok",
    )
    .await;
    assert!(
        output.to_ascii_lowercase().contains("rove provider smoke ok"),
        "unexpected provider smoke output: {output}"
    );
}
```

- [ ] **Step 2: Run default smoke tests with no gates**

Run:

```powershell
cargo test --test provider_smoke
```

Expected: PASS quickly. All tests return early unless their env gate is set.

- [ ] **Step 3: Create provider smoke documentation**

Create `docs/runtime/provider-smoke.md`:

```markdown
# Provider Smoke

Provider smoke tests are opt-in checks for real model endpoints. They are not part of the default deterministic test suite because they require credentials, network access, local Ollama availability, or provider-specific quota.

## Default behavior

```powershell
cargo test --test provider_smoke
```

With no smoke gates enabled, the tests exit early and should pass.

## OpenAI-compatible

```powershell
$env:ROVE_PROVIDER_SMOKE_OPENAI = "1"
$env:OPENAI_API_KEY = "<secret>"
$env:ROVE_PROVIDER_SMOKE_OPENAI_MODEL = "gpt-4.1-mini"
cargo test --test provider_smoke openai_compatible_real_provider_smoke_when_enabled -- --exact --nocapture
```

Set `OPENAI_API_BASE` when testing a compatible endpoint that is not OpenAI.

## Anthropic

```powershell
$env:ROVE_PROVIDER_SMOKE_ANTHROPIC = "1"
$env:ANTHROPIC_API_KEY = "<secret>"
$env:ROVE_PROVIDER_SMOKE_ANTHROPIC_MODEL = "claude-3-5-haiku-latest"
cargo test --test provider_smoke anthropic_real_provider_smoke_when_enabled -- --exact --nocapture
```

## Ollama

Start Ollama locally before running the smoke:

```powershell
ollama serve
```

Then run:

```powershell
$env:ROVE_PROVIDER_SMOKE_OLLAMA = "1"
$env:ROVE_PROVIDER_SMOKE_OLLAMA_MODEL = "llama3.2"
cargo test --test provider_smoke ollama_real_provider_smoke_when_enabled -- --exact --nocapture
```

## Expected result

Each enabled smoke asks the provider for a short deterministic phrase. Passing the smoke proves the configured provider can be reached, stream events can be normalized, and the engine can complete a minimal run through the real provider path.
```

- [ ] **Step 4: Link provider smoke docs**

Add a `provider-smoke.md` row to `docs/runtime/README.md`:

```markdown
| [provider-smoke.md](provider-smoke.md) | Opt-in real-provider verification for OpenAI-compatible, Anthropic, and Ollama paths. |
```

Add this paragraph to `docs/runtime/implementation-guide.md` under Testing and Verification:

```markdown
Real provider smoke tests are opt-in and documented in `docs/runtime/provider-smoke.md`. They are intentionally excluded from default CI because they require credentials, network access, local Ollama availability, or provider quota.
```

- [ ] **Step 5: Run verification**

Run:

```powershell
cargo test --test provider_smoke
cargo test --test code_hygiene
```

Expected: PASS.

- [ ] **Step 6: Commit**

```powershell
git add docs/runtime/provider-smoke.md docs/runtime/README.md docs/runtime/implementation-guide.md tests/provider_smoke.rs
git commit -m "test: add opt-in provider smoke checks"
```

---

## Work Package 3: Add API Endpoints for Historical Runs and Reports

### Problem

The API can stream a live job and replay one job by id, but the Web cannot discover old runs or fetch `report.json` without knowing filesystem paths. That keeps the Web UI from becoming a usable workbench for completed work.

### Implementation

Expose read-only API endpoints backed by existing SQLite records and report artifacts:

- `GET /runs?limit=50`
  - Returns recent runs with identity, status, paths, last event seq, and report availability.
- `GET /runs/{run_id}/report`
  - Loads and returns the indexed `report.json`.

Do not expose arbitrary file paths. The report endpoint should use the indexed report path from `StateIndex::report_record`, then deserialize as `RunReport`.

### Acceptance Standard

- API tests can create a fake job, wait for completion, list runs, and fetch its report.
- API tests can construct a fresh `ApiState` over the same workspace and still list/fetch completed historical state.
- Unknown run ids return 404.
- No write or destructive capability is added.

### Task 3: Add State Index Listing

**Files:**
- Modify: `src/state/index.rs`
- Test: `tests/api.rs`

- [ ] **Step 1: Add failing API test for run listing**

Add this test to `tests/api.rs`:

```rust
#[tokio::test]
async fn api_lists_completed_runs_after_job_finishes() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"message":"listable run","model":"fake","approval":"auto"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let state = wait_for_done(app.clone(), created.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Done);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/runs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let runs = body["runs"].as_array().expect("runs array");
    assert!(
        runs.iter().any(|run| run["run_id"] == created.run_id.to_string()),
        "completed run should appear in /runs response: {body}"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```powershell
cargo test --test api api_lists_completed_runs_after_job_finishes -- --exact
```

Expected: FAIL with 404 or route not found for `/runs`.

- [ ] **Step 3: Add index record type and list method**

In `src/state/index.rs`, add:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunListRecord {
    pub run_id: RunId,
    pub session_id: SessionId,
    pub job_id: JobId,
    pub status: String,
    pub run_dir: PathBuf,
    pub trace_path: PathBuf,
    pub task_state_path: Option<PathBuf>,
    pub report_path: Option<PathBuf>,
    pub last_event_seq: u64,
}
```

Add async and sync methods on `StateIndex`:

```rust
pub async fn list_run_records_async(&self, limit: usize) -> std::io::Result<Vec<RunListRecord>> {
    let index = self.clone();
    tokio::task::spawn_blocking(move || index.list_run_records(limit))
        .await
        .map_err(std::io::Error::other)?
}

pub fn list_run_records(&self, limit: usize) -> std::io::Result<Vec<RunListRecord>> {
    let conn = self.connect()?;
    let limit = limit.clamp(1, 200) as i64;
    let mut stmt = conn
        .prepare(
            r#"
            SELECT run_id, session_id, job_id, status, run_dir, trace_path,
                   task_state_path, report_path, last_event_seq
            FROM runs
            ORDER BY updated_at DESC, started_at DESC, run_id DESC
            LIMIT ?1
            "#,
        )
        .map_err(io_other)?;
    let rows = stmt
        .query_map(params![limit], run_list_record_from_row)
        .map_err(io_other)?;
    let mut records = Vec::new();
    for row in rows {
        records.push(row.map_err(io_other)?);
    }
    Ok(records)
}
```

Add row mapper:

```rust
fn run_list_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunListRecord> {
    Ok(RunListRecord {
        run_id: run_id_from_row(row, 0)?,
        session_id: session_id_from_row(row, 1)?,
        job_id: job_id_from_row(row, 2)?,
        status: row.get(3)?,
        run_dir: PathBuf::from(row.get::<_, String>(4)?),
        trace_path: PathBuf::from(row.get::<_, String>(5)?),
        task_state_path: row.get::<_, Option<String>>(6)?.map(PathBuf::from),
        report_path: row.get::<_, Option<String>>(7)?.map(PathBuf::from),
        last_event_seq: row.get::<_, i64>(8)?.max(0) as u64,
    })
}
```

- [ ] **Step 4: Run compile check**

Run:

```powershell
cargo test --test api api_lists_completed_runs_after_job_finishes -- --exact
```

Expected: still FAIL because API route is not added yet, but code compiles.

### Task 4: Add API Run List and Report Endpoints

**Files:**
- Modify: `src/interfaces/api/mod.rs`
- Modify: `src/state/store.rs`
- Test: `tests/api.rs`

- [ ] **Step 1: Add response types and routes**

In `src/interfaces/api/mod.rs`, add response structs:

```rust
#[derive(Debug, Serialize, Deserialize)]
pub struct ListRunsResponse {
    pub runs: Vec<RunSummaryResponse>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RunSummaryResponse {
    pub run_id: RunId,
    pub session_id: SessionId,
    pub job_id: JobId,
    pub status: RunStatus,
    pub last_event_seq: u64,
    pub has_report: bool,
}

#[derive(Debug, Deserialize)]
struct RunsQuery {
    limit: Option<usize>,
}
```

Add routes in `router`:

```rust
.route("/runs", get(list_runs))
.route("/runs/{run_id}/report", get(run_report))
```

- [ ] **Step 2: Implement handlers**

Add handlers:

```rust
async fn list_runs(
    State(state): State<ApiState>,
    Query(query): Query<RunsQuery>,
) -> Result<Json<ListRunsResponse>, ApiError> {
    let state_store = state_store_for_api(&state);
    let records = state_store
        .index
        .list_run_records_async(query.limit.unwrap_or(50))
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(ListRunsResponse {
        runs: records
            .into_iter()
            .map(|record| RunSummaryResponse {
                run_id: record.run_id,
                session_id: record.session_id,
                job_id: record.job_id,
                status: run_status_from_index(&record.status),
                last_event_seq: record.last_event_seq,
                has_report: record.report_path.is_some(),
            })
            .collect(),
    }))
}

async fn run_report(
    State(state): State<ApiState>,
    Path(run_id): Path<RunId>,
) -> Result<Json<crate::state::report::RunReport>, ApiError> {
    let state_store = state_store_for_api(&state);
    let report = state_store
        .load_report(run_id)
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => ApiError::not_found("run report not found"),
            _ => ApiError::internal(err),
        })?;
    Ok(Json(report))
}
```

- [ ] **Step 3: Add report loader**

In `src/state/store.rs`, add:

```rust
pub async fn load_report(&self, run_id: RunId) -> std::io::Result<RunReport> {
    let Some(record) = self.index.report_record(run_id)? else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("report not found for run {run_id}"),
        ));
    };
    self.load_report_path(&record.path).await
}
```

- [ ] **Step 4: Add report API tests**

Add these tests to `tests/api.rs`:

```rust
#[tokio::test]
async fn api_fetches_completed_run_report() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"message":"reportable run","model":"fake","approval":"auto"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let state = wait_for_done(app.clone(), created.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Done);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/runs/{}/report", created.run_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(report["run_id"], created.run_id.to_string());
    assert_eq!(report["job_id"], created.job_id.to_string());
    assert_eq!(report["status"], "success");
}

#[tokio::test]
async fn api_returns_404_for_missing_run_report() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let app = router(ApiState::new(workspace, test_config()));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/runs/01ARZ3NDEKTSV4RRFFQ69G5FAV/report")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn api_lists_and_fetches_run_report_after_restart() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let config = test_config();
    let app = router(ApiState::new(workspace.clone(), config.clone()));

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"message":"restart reportable run","model":"fake","approval":"auto"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: CreateJobResponse = serde_json::from_slice(&body).unwrap();

    let state = wait_for_done(app.clone(), created.job_id.to_string()).await;
    assert_eq!(state.status, RunStatus::Done);

    let restarted = router(ApiState::new(workspace, config));
    let runs = restarted
        .clone()
        .oneshot(
            Request::builder()
                .uri("/runs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(runs.status(), StatusCode::OK);
    let body = axum::body::to_bytes(runs.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(body["runs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|run| run["run_id"] == created.run_id.to_string()));

    let report = restarted
        .oneshot(
            Request::builder()
                .uri(format!("/runs/{}/report", created.run_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(report.status(), StatusCode::OK);
}
```

- [ ] **Step 5: Run API tests**

Run:

```powershell
cargo test --test api api_lists_completed_runs_after_job_finishes -- --exact
cargo test --test api api_fetches_completed_run_report -- --exact
```

Expected: PASS.

- [ ] **Step 6: Run broader API suite**

Run:

```powershell
cargo test --test api
```

Expected: PASS.

- [ ] **Step 7: Commit**

```powershell
git add src/state/index.rs src/state/store.rs src/interfaces/api/mod.rs tests/api.rs
git commit -m "feat: expose historical run reports"
```

---

## Work Package 4: Add Web History and Report Viewing

### Problem

The Web workbench can operate a live run but cannot review completed work. Users need a way to see recent runs, pick one, and inspect report metadata and output without leaving the browser or reading `.rove/runs` manually.

### Implementation

Add a compact read-only history section to the existing right rail or signal area:

- Load recent runs from `GET /runs` on page load and after a job completes.
- Show run id, status, last event seq, and report availability.
- Clicking a run with a report calls `GET /runs/{run_id}/report`.
- Show report detail: model id, workspace kind/root, status, termination reason, steps, tool calls/failures, mutations, final output, timestamp.

Keep this inside the current workbench page. Do not add routing, auth changes, or separate pages.

### Acceptance Standard

- Web unit tests cover `listRuns` and `fetchRunReport`.
- Web reducer/component state can display selected report data.
- `pnpm test`, `pnpm typecheck`, and `pnpm build` pass.
- Optional Playwright test confirms report history is visible with mocked API responses.

### Task 5: Add Web Types and Client Methods

**Files:**
- Modify: `web-ui/lib/rove-types.ts`
- Modify: `web-ui/lib/rove-client.ts`
- Modify: `web-ui/lib/rove-client.test.ts`

- [ ] **Step 1: Add failing client tests**

Add to `web-ui/lib/rove-client.test.ts`:

```ts
import { fetchRunReport, listRuns } from "./rove-client";

it("fetches recent runs", async () => {
  const fetchMock = vi.fn().mockResolvedValue({
    ok: true,
    json: async () => ({
      runs: [
        {
          run_id: "run-1",
          session_id: "session-1",
          job_id: "job-1",
          status: "done",
          last_event_seq: 5,
          has_report: true,
        },
      ],
    }),
  });
  vi.stubGlobal("fetch", fetchMock);

  const result = await listRuns(25);

  expect(fetchMock).toHaveBeenCalledWith("/api/runs?limit=25");
  expect(result.runs[0].run_id).toBe("run-1");
});

it("fetches a run report", async () => {
  const fetchMock = vi.fn().mockResolvedValue({
    ok: true,
    json: async () => ({
      session_id: "session-1",
      job_id: "job-1",
      run_id: "run-1",
      workspace_root: "D:/workspace",
      workspace_kind: "Folder",
      model_id: "fake",
      status: "success",
      termination_reason: "Final",
      steps: 1,
      total_usage: { prompt_tokens: 0, completion_tokens: 0, total_tokens: 0 },
      tool_calls: 0,
      tool_failures: 0,
      tool_mutations: [],
      output: "done",
      timestamp: "2026-05-30T00:00:00Z",
    }),
  });
  vi.stubGlobal("fetch", fetchMock);

  const result = await fetchRunReport("run/1");

  expect(fetchMock).toHaveBeenCalledWith("/api/runs/run%2F1/report");
  expect(result.output).toBe("done");
});
```

- [ ] **Step 2: Run tests to verify failure**

Run:

```powershell
cd web-ui
pnpm test -- rove-client.test.ts
```

Expected: FAIL because `listRuns` and `fetchRunReport` are not exported.

- [ ] **Step 3: Add types**

In `web-ui/lib/rove-types.ts`, add:

```ts
export interface ListRunsResponse {
  runs: RunSummary[];
}

export interface RunSummary {
  run_id: string;
  session_id: string;
  job_id: string;
  status: RunStatus;
  last_event_seq: number;
  has_report: boolean;
}

export interface RunReport {
  session_id: string;
  job_id: string;
  run_id: string;
  workspace_root: string;
  workspace_kind: string;
  model_id: string;
  status: string;
  termination_reason: string;
  steps: number;
  total_usage: Usage;
  tool_calls: number;
  tool_failures: number;
  tool_mutations: ToolMutation[];
  output?: string | null;
  timestamp: string;
}
```

- [ ] **Step 4: Add client functions**

In `web-ui/lib/rove-client.ts`, import the new types and add:

```ts
import type { ListRunsResponse, RunReport } from "./rove-types";

export async function listRuns(limit = 50): Promise<ListRunsResponse> {
  const response = await fetch(apiUrl(`/runs?limit=${encodeURIComponent(String(limit))}`));
  return parseJson<ListRunsResponse>(response);
}

export async function fetchRunReport(runId: string): Promise<RunReport> {
  const response = await fetch(apiUrl(`/runs/${encodeURIComponent(runId)}/report`));
  return parseJson<RunReport>(response);
}
```

- [ ] **Step 5: Run client tests**

Run:

```powershell
cd web-ui
pnpm test -- rove-client.test.ts
```

Expected: PASS.

### Task 6: Add Workbench History UI

**Files:**
- Modify: `web-ui/components/rove-workbench.tsx`
- Modify: `web-ui/lib/rove-state.ts` if storing selected report in reducer
- Test: `web-ui/lib/rove-state.test.ts` if reducer changes

- [ ] **Step 1: Add local component state**

In `RoveWorkbench`, add imports:

```ts
import type { RunReport, RunSummary } from "../lib/rove-types";
import { fetchRunReport, listRuns } from "../lib/rove-client";
```

Update existing `rove-client` import instead of creating duplicate imports.

Add state:

```ts
const [runs, setRuns] = useState<RunSummary[]>([]);
const [selectedReport, setSelectedReport] = useState<RunReport | null>(null);
const [historyBusy, setHistoryBusy] = useState(false);
const [historyError, setHistoryError] = useState<string | null>(null);
```

- [ ] **Step 2: Load history on mount and after completion**

Add:

```ts
useEffect(() => {
  void refreshRuns();
}, []);

async function refreshRuns() {
  setHistoryBusy(true);
  setHistoryError(null);
  try {
    const result = await listRuns(25);
    setRuns(result.runs);
  } catch (error) {
    setHistoryError(describeError(error));
  } finally {
    setHistoryBusy(false);
  }
}

async function handleReportSelect(run: RunSummary) {
  if (!run.has_report) {
    setSelectedReport(null);
    return;
  }
  setHistoryBusy(true);
  setHistoryError(null);
  try {
    setSelectedReport(await fetchRunReport(run.run_id));
  } catch (error) {
    setHistoryError(describeError(error));
  } finally {
    setHistoryBusy(false);
  }
}
```

After handling a `run_completed` event, call:

```ts
void refreshRuns();
```

After `handleCancel` terminal sync, also call:

```ts
void refreshRuns();
```

- [ ] **Step 3: Add history panel**

In the right rail, add an `InspectorSection` before `Trace`:

```tsx
<InspectorSection title="History" icon={<ActivityLogIcon />}>
  {historyError ? <div className="empty-block">{historyError}</div> : null}
  {historyBusy && runs.length === 0 ? (
    <EmptyBlock label="Loading runs" busy />
  ) : runs.length ? (
    <div className="stack-list">
      {runs.map((run) => (
        <button
          key={run.run_id}
          type="button"
          className="stack-row stack-row--button"
          onClick={() => void handleReportSelect(run)}
          disabled={!run.has_report}
        >
          <div className="stack-row__header">
            <strong>{shortId(run.run_id)}</strong>
            <span>{run.status}</span>
          </div>
          <p>
            {shortId(run.job_id)} / {run.last_event_seq} events
          </p>
        </button>
      ))}
    </div>
  ) : (
    <EmptyBlock label="No historical runs" busy={historyBusy} />
  )}
</InspectorSection>
```

- [ ] **Step 4: Add report detail panel**

Add below the History section:

```tsx
<InspectorSection title="Report" icon={<FileTextIcon />}>
  {selectedReport ? (
    <div className="report-panel">
      <SummaryRow label="run" value={shortId(selectedReport.run_id)} />
      <SummaryRow label="model" value={selectedReport.model_id} />
      <SummaryRow label="status" value={selectedReport.status} />
      <SummaryRow label="reason" value={selectedReport.termination_reason} />
      <SummaryRow label="steps" value={String(selectedReport.steps)} />
      <SummaryRow label="tools" value={`${selectedReport.tool_calls}/${selectedReport.tool_failures}`} />
      <div className="report-panel__output">
        <span>output</span>
        <p>{selectedReport.output ?? "No output"}</p>
      </div>
    </div>
  ) : (
    <EmptyBlock label="Select a run" busy={historyBusy} />
  )}
</InspectorSection>
```

- [ ] **Step 5: Add CSS for button rows and report panel**

In `web-ui/app/globals.css`, add styles consistent with existing `stack-row` styling:

```css
.stack-row--button {
  width: 100%;
  border: 0;
  text-align: left;
  cursor: pointer;
}

.stack-row--button:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

.report-panel {
  display: grid;
  gap: 10px;
}

.report-panel__output {
  display: grid;
  gap: 4px;
}

.report-panel__output span {
  color: var(--text-muted);
  font-size: 12px;
}

.report-panel__output p {
  margin: 0;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
```

Use existing CSS variables/classes; if the names differ, adapt to the local CSS without introducing a new visual theme.

- [ ] **Step 6: Run Web checks**

Run:

```powershell
cd web-ui
pnpm test
pnpm typecheck
pnpm build
```

Expected: all PASS.

- [ ] **Step 7: Commit**

```powershell
git add web-ui/lib/rove-types.ts web-ui/lib/rove-client.ts web-ui/lib/rove-client.test.ts web-ui/components/rove-workbench.tsx web-ui/app/globals.css
git commit -m "feat: show historical run reports in workbench"
```

---

## Final Verification

Run the default backend gate:

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Run the Web gate:

```powershell
cd web-ui
pnpm test
pnpm typecheck
pnpm build
```

Run deterministic benchmark smoke:

```powershell
cargo run --bin rove-bench -- --suite benchmarks/agent-smoke.json --output-dir .rove/bench
```

Run provider smoke default path:

```powershell
cargo test --test provider_smoke
```

Expected:

- Default tests pass without credentials.
- `docs/runtime/mvp-definition.md` exists and is linked from runtime docs and root README.
- `GET /runs` returns recent persisted runs.
- `GET /runs/{run_id}/report` returns `report.json` for completed runs.
- Web workbench can list historical runs and show a selected report.

---

## Self-Review Checklist

- Spec coverage: all four requested recommendations are represented by work packages and acceptance standards.
- Scope: Browser/Desktop workspaces and deeper RAG runtime integration are explicitly excluded.
- Testability: each work package includes focused failing tests before implementation and final verification commands.
- Risk: provider smoke remains opt-in, so default CI remains deterministic and credential-free.
