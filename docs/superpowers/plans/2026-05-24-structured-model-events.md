# Structured Model Events Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move rove's model layer from text-only `StreamChunk` semantics to structured `ModelEvent` semantics so native provider tool-use can flow into the existing tool execution pipeline without changing the public job/SSE/Web protocol.

**Architecture:** Add `ModelEvent` in `src/models/traits.rs` and keep `StreamChunk` as a compatibility helper only if needed during migration. Provider adapters emit normalized text, tool-use, usage, and done events; routing treats first meaningful content/tool events as the fallback commit point; engine consumes a normalized `ModelTurn` assembled from model events and then reuses the existing approval/executor path.

**Tech Stack:** Rust, async streams (`futures::stream::BoxStream`, `async_stream`), serde_json, existing rove engine/events/tests.

---

### Task 1: Add Structured ModelEvent Contract

**Files:**
- Modify: `src/models/traits.rs`
- Modify: `src/models/fake.rs`
- Modify: `src/models/routing.rs`
- Test: `src/models/routing.rs`

- [x] **Step 1: Write failing routing tests**

Add tests in `src/models/routing.rs` that use clients emitting `ModelEvent::Usage`, `ModelEvent::TextDelta`, and `ModelEvent::ToolUseStart`. The tests must assert:

```rust
#[tokio::test]
async fn usage_only_first_event_does_not_commit_routing_provider() {
    // primary emits Usage then errors; fallback emits TextDelta.
    // Expected: fallback is called and its text is returned.
}

#[tokio::test]
async fn tool_use_start_commits_routing_provider() {
    // primary emits ToolUseStart then errors; fallback is not called.
    // Expected: the tool-use event and then the original error are returned.
}
```

- [x] **Step 2: Run tests and verify RED**

Run:

```powershell
cargo test models::routing::tests::usage_only_first_event_does_not_commit_routing_provider models::routing::tests::tool_use_start_commits_routing_provider
```

Expected: compile failure because `ModelEvent` does not exist and `ModelClient::stream` still returns `StreamChunk`.

- [x] **Step 3: Implement minimal contract migration**

Change `src/models/traits.rs` to:

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum ModelEvent {
    TextDelta { text: String },
    ThinkingDelta { text: String },
    ToolUseStart { id: String, name: String },
    ToolUseDelta { id: String, args_delta: String },
    ToolUseDone { id: String, name: String, args: serde_json::Value },
    Usage { usage: Usage },
    Done,
}

pub trait ModelClient: Send + Sync {
    fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>>;
    fn model_id(&self) -> &str;
}
```

Update fake/routing/tests to use `ModelEvent::TextDelta` and `ModelEvent::Usage`.

- [x] **Step 4: Run tests and verify GREEN**

Run:

```powershell
cargo test models::routing
```

Expected: routing tests pass.

### Task 2: Add Provider Parser Tests For Native Tool Use

**Files:**
- Modify: `src/models/openai.rs`
- Modify: `src/models/anthropic.rs`
- Modify: `src/models/ollama.rs`

- [x] **Step 1: Write failing parser tests**

Add pure parser helpers and tests:

```rust
#[test]
fn openai_delta_tool_call_events_are_normalized() {
    let mut state = OpenAiToolCallState::default();
    let events = normalize_openai_chat_chunk(&mut state, OPENAI_TOOL_DELTA_JSON).unwrap();
    assert!(events.iter().any(|event| matches!(event, ModelEvent::ToolUseStart { name, .. } if name == "fs_read")));
}

#[test]
fn anthropic_tool_use_blocks_are_normalized() {
    let events = normalize_anthropic_event(ANTHROPIC_TOOL_USE_JSON).unwrap();
    assert!(events.iter().any(|event| matches!(event, ModelEvent::ToolUseDone { name, args, .. } if name == "fs_read" && args["path"] == "Cargo.toml")));
}

#[test]
fn ollama_tool_calls_are_normalized() {
    let events = normalize_ollama_chat_line(OLLAMA_TOOL_CALL_JSON).unwrap();
    assert!(events.iter().any(|event| matches!(event, ModelEvent::ToolUseDone { name, args, .. } if name == "fs_read" && args["path"] == "Cargo.toml")));
}
```

- [x] **Step 2: Run tests and verify RED**

Run each provider module test:

```powershell
cargo test models::openai::tests::openai_delta_tool_call_events_are_normalized
cargo test models::anthropic::tests::anthropic_tool_use_blocks_are_normalized
cargo test models::ollama::tests::ollama_tool_calls_are_normalized
```

Expected: compile failure because parser helpers do not exist.

- [x] **Step 3: Implement provider normalization**

Implement provider-local helper functions that map provider JSON into `Vec<ModelEvent>`. Stream loops call these helpers and yield each returned event. Preserve existing text and usage behavior as `TextDelta` / `Usage`.

- [x] **Step 4: Run provider tests and verify GREEN**

Run:

```powershell
cargo test models::openai models::anthropic models::ollama
```

Expected: provider tests pass.

### Task 3: Teach Engine To Consume Native ToolUseDone

**Files:**
- Modify: `src/core/engine.rs`
- Modify: `src/core/planner.rs`
- Test: `tests/e2e.rs`

- [x] **Step 1: Write failing engine e2e test**

Add a fake model client that emits:

```rust
ModelEvent::ToolUseStart { id: "native-call-1".into(), name: "echo".into() },
ModelEvent::ToolUseDone {
    id: "native-call-1".into(),
    name: "echo".into(),
    args: serde_json::json!({ "message": "native hello" }),
},
ModelEvent::Usage { usage: Usage::default() },
ModelEvent::Done,
```

Assert that an engine run emits `StreamEvent::ToolCallStarted` and `StreamEvent::ToolCallCompleted` with output containing `native hello`.

- [x] **Step 2: Run test and verify RED**

Run:

```powershell
cargo test --test e2e engine_executes_native_model_tool_use
```

Expected: compile failure or failing assertion because engine does not consume `ModelEvent::ToolUseDone`.

- [x] **Step 3: Implement `ModelTurn` assembly**

In `src/core/engine.rs`, collect model events into:

```rust
struct ModelTurn {
    text: String,
    usage: Usage,
    tool_call: Option<NativeToolCall>,
}

struct NativeToolCall {
    call_id: CallId,
    provider_id: String,
    name: String,
    args: serde_json::Value,
}
```

Emit existing `StreamEvent::LlmChunk` for `TextDelta`, existing `StreamEvent::LlmMessage` after the turn, and route `NativeToolCall` through the existing tool approval/execution branch before falling back to `parse_action(&text)`.

- [x] **Step 4: Keep planner text-only**

Update `src/core/planner.rs` so planner concatenates only `ModelEvent::TextDelta` and ignores tool-use events.

- [x] **Step 5: Run e2e test and verify GREEN**

Run:

```powershell
cargo test --test e2e engine_executes_native_model_tool_use
```

Expected: test passes.

### Task 4: Regression Gates

**Files:**
- No new files required.

- [x] **Step 1: Run Rust formatting and tests**

Run:

```powershell
cargo fmt --all --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: all pass.

- [x] **Step 2: Run Web regression checks**

Run:

```powershell
Set-Location web-ui
npm test
npm run typecheck
npm run build
```

Expected: all pass; public Web protocol should be unchanged.

- [x] **Step 3: Commit implementation**

Run:

```powershell
git status --short
git add src tests docs
git commit -m "feat: add structured model events"
```

Expected: commit contains Phase 2A only.
