# Runtime Guide Gap Closure Implementation Plan

> **For implementers:** Execute this plan task by task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the first implementation gaps documented in `docs/runtime/implementation-guide.md` while keeping each fix testable and isolated.

**Architecture:** Treat `implementation-guide.md` section 22 as the current requirements source. Fix small contract/path gaps first, then leave larger runtime architecture gaps for follow-up tasks. Preserve the current CLI/API/Web/Core layering.

**Tech Stack:** Rust 2024, cargo, tokio, axum, serde, Vitest, TypeScript.

---

## File Structure

- Modify `Cargo.toml` to make `rove` the default binary so documented `cargo run -- ...` commands work.
- Modify `web-ui/lib/rove-types.ts` and `web-ui/lib/rove-state.ts` to accept and render `interrupted` job state.
- Modify `web-ui/lib/rove-state.test.ts` with a regression test for interrupted historical job sync.
- Modify `src/tools/memory.rs` so memory tools can write to the configured state directory through `ToolContext.workspace.state_dir`.
- Modify `tests/memory_tool.rs` with a regression test for custom state directories.
- Modify `src/interfaces/api/mod.rs` and `src/interfaces/cli/oneshot.rs` to support API resume through the same `StateStore` path used by CLI.
- Modify `tests/api.rs` with a regression test that API can resume the latest task state.
- Update `docs/runtime/implementation-guide.md` known gaps after each closed item.
- Update `README.md` only if `default-run` is not sufficient.

## Task 1: Cargo Default Run

**Files:**
- Modify: `Cargo.toml`

- [x] **Step 1: Verify the documented command currently fails**

Run:

```powershell
cargo run -- dump-config
```

Expected: FAIL with Cargo saying it cannot determine which binary to run.

- [x] **Step 2: Add default binary metadata**

Add this field under `[package]`:

```toml
default-run = "rove"
```

- [x] **Step 3: Verify the documented commands work**

Run:

```powershell
cargo run -- dump-config
cargo run -- --model fake "echo hello from rove"
```

Expected: both commands execute the `rove` binary.

## Task 2: Web Interrupted Status

**Files:**
- Modify: `web-ui/lib/rove-types.ts`
- Modify: `web-ui/lib/rove-state.ts`
- Modify: `web-ui/lib/rove-state.test.ts`

- [x] **Step 1: Write the failing Web reducer test**

Add a Vitest case that dispatches `job_state_synced` with:

```ts
status: "interrupted"
```

Expected reducer state:

```ts
busy === false
statusText === "Run interrupted"
```

- [x] **Step 2: Verify RED**

Run:

```powershell
cd web-ui
npm test -- rove-state
```

Expected: FAIL because `"interrupted"` is not part of `RunStatus` and is not handled by `statusText` / `statusDetail`.

- [x] **Step 3: Implement status parity**

Add `"interrupted"` to `RunStatus` and handle it in both status helper functions.

- [x] **Step 4: Verify GREEN**

Run:

```powershell
cd web-ui
npm test -- rove-state
npm run typecheck
```

Expected: PASS.

## Task 3: Memory Tools Honor Workspace State Directory

**Files:**
- Modify: `src/tools/memory.rs`
- Modify: `tests/memory_tool.rs`

- [x] **Step 1: Write the failing memory path test**

Add a test that creates a `Workspace`, overrides `workspace.state_dir` to a custom directory, runs `save_memory`, and asserts the topic and `MEMORY.md` are written under the custom state directory, not `workspace.root/.rove`.

- [x] **Step 2: Verify RED**

Run:

```powershell
cargo test --test memory_tool save_memory_writes_to_configured_workspace_state_dir -- --exact
```

Expected: FAIL because `SaveMemoryTool` currently writes to `self.root.join(".rove").join("memory")`.

- [x] **Step 3: Route writes through `ToolContext.workspace.state_dir`**

In memory tools, replace hard-coded `self.root.join(".rove").join("memory")` with a helper that resolves:

```rust
ctx.workspace.state_dir.join("memory")
```

Keep the existing `root` field only where needed for compatibility, or remove it if no longer needed by constructors.

- [x] **Step 4: Verify GREEN**

Run:

```powershell
cargo test --test memory_tool save_memory_writes_to_configured_workspace_state_dir -- --exact
cargo test --test memory_tool
```

Expected: PASS.

## Task 4: API Resume Latest

**Files:**
- Modify: `src/interfaces/api/mod.rs`
- Modify: `tests/api.rs`

- [x] **Step 1: Write the failing API resume test**

Create a completed fake-model job, then create a second job with a new request field:

```json
{
  "message": "continue",
  "model": "fake",
  "resume": "latest"
}
```

Assert the second run's persisted `task_state.json` has the same `session_id` and `job_id` as the latest prior state, and that its history/checkpoint data includes resume context.

- [x] **Step 2: Verify RED**

Run:

```powershell
cargo test --test api api_can_resume_latest_task_state -- --exact
```

Expected: FAIL because `CreateJobRequest` has no `resume` field and API always starts fresh.

- [x] **Step 3: Implement API resume resolution**

Add:

```rust
pub resume: Option<String>
```

to `CreateJobRequest`, and reuse the same resume resolution semantics as CLI:

- absent: fresh session/job/run;
- `"latest"`: load latest task state from `StateStore`;
- ULID string: load exact `RunId`;
- invalid string: return bad request.

For resumed API jobs, keep the new `run_id`, but use the resumed `session_id` and `job_id`, and pass the loaded `TaskState` to `run.request`.

- [x] **Step 4: Verify GREEN**

Run:

```powershell
cargo test --test api api_can_resume_latest_task_state -- --exact
cargo test --test api
```

Expected: PASS.

## Task 5: Documentation Sync

**Files:**
- Modify: `docs/runtime/implementation-guide.md`
- Modify: `README.md` if needed

- [x] **Step 1: Update Known Gaps**

Remove or reword closed items:

- README command drift
- Web status parity
- Configured memory paths
- API resume parity

- [x] **Step 2: Verify stale text is gone**

Run:

```powershell
rg -n "README command drift|Web status parity|Configured memory paths are not fully honored|API resume parity" docs/runtime/implementation-guide.md
```

Expected: no matches for closed-gap wording.

## Final Verification

- [x] `cargo fmt --all --check`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test`
- [x] `cd web-ui; npm test`
- [x] `cd web-ui; npm run typecheck`
- [x] `git status --short --branch`
