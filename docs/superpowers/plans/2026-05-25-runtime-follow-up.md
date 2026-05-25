# Runtime Follow-Up Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the remaining runtime gaps identified after `docs/runtime/` became authoritative: API security middleware, explicit state maintenance, automatic compaction behavior, RAG 3B/3D boundaries, and documentation cleanup.

**Architecture:** Keep the existing local-first runtime shape. Add narrow modules at subsystem boundaries instead of rewriting core files: API middleware in `src/interfaces/api/security.rs`, state maintenance methods behind `StateStore`/`StateIndex`, compaction policy helpers near `core::context`/`state::artifacts`, and RAG prompt/code chunking helpers under `src/tools/rag/`.

**Tech Stack:** Rust 2024, Axum/Tower, Tokio, rusqlite, Clap, serde, existing feature-gated RAG modules.

---

## Files

- Create: `src/interfaces/api/security.rs`
- Modify: `src/interfaces/api/mod.rs`
- Modify: `tests/api.rs`
- Modify: `src/state/index.rs`
- Modify: `src/state/store.rs`
- Create: `src/interfaces/cli/state.rs`
- Modify: `src/interfaces/cli/mod.rs`
- Modify: `src/interfaces/cli/args.rs`
- Modify: `src/main.rs`
- Modify: `tests/e2e.rs`
- Modify: `src/core/types.rs`
- Modify: `src/core/context.rs`
- Modify: `src/state/artifacts.rs`
- Modify: `tests/e2e.rs`
- Create: `src/tools/rag/prompt.rs`
- Modify: `src/tools/rag/mod.rs`
- Modify: `src/tools/rag/ingest/chunking.rs`
- Modify: `tests/rag.rs`
- Modify: `docs/00-README.md`
- Modify: `docs/runtime/README.md`
- Modify: `docs/runtime/implementation-status.md`
- Modify: `docs/04-架构与路线图.md`
- Modify: `docs/05-下一步-统一执行内核.md`
- Modify: `docs/06-请求生命周期.md`
- Delete: `docs/GOAL.md`

## Task 1: API Security Middleware

- [x] Add RED tests in `tests/api.rs`:
  - missing `Authorization: Bearer <token>` returns `401` when `config.api.token_auth` is set.
  - matching bearer token allows the request.
  - disallowed `Origin` returns `403` when `config.api.cors_origins` is non-empty.
  - allowed `Origin` adds `access-control-allow-origin`.
  - configured `rate_limit_per_minute = Some(2)` returns `429` on the third request from the same client bucket.
- [x] Run the focused tests and confirm they fail because middleware is missing:
  - `cargo test --test api api_rejects_missing_bearer_token_when_configured`
  - `cargo test --test api api_allows_matching_bearer_token`
  - `cargo test --test api api_rejects_disallowed_cors_origin`
  - `cargo test --test api api_allows_configured_cors_origin_and_sets_headers`
  - `cargo test --test api api_rate_limits_requests_when_configured`
- [x] Implement `src/interfaces/api/security.rs` as a Tower middleware using existing `AppConfig.api` fields.
- [x] Wire it in `router(state)` without changing existing route handlers.
- [x] Run `cargo test --test api` and `cargo check --all-targets`.

## Task 2: State Maintenance

- [x] Add RED tests in `tests/e2e.rs`:
  - explicit repair indexes legacy `runs/*/task_state.json` artifacts without relying on a list/load call.
  - cleanup removes expired jobs/runs/events/task-state index rows when `ttl_expires_at` is in the past, while leaving non-expired rows intact.
- [x] Add `StateStore::repair_index()` that reuses the existing lazy import logic but returns an import count.
- [x] Add `StateIndex::cleanup_expired(now)` and async wrapper, deleting dependent rows in SQLite transaction order.
- [x] Add `rove state repair` and `rove state cleanup` subcommands in `src/interfaces/cli/state.rs`.
- [x] Run `cargo test --test e2e state_repair_imports_legacy_task_states state_cleanup_removes_expired_index_rows`.

## Task 3: Compaction Auto Policy

- [x] Add RED tests around checkpoint behavior in `tests/e2e.rs`:
  - checkpoint metadata records whether compaction was automatic/deterministic and when the compacted history exceeded the soft limit.
  - repeated compaction failures increment a circuit counter and mark auto compaction disabled after the configured threshold.
- [x] Extend `PromptCheckpoint` with serde-defaulted metadata so old `task_state.json` files still deserialize.
- [x] Add a small compaction policy helper that uses `ContextBuild` estimates and existing checkpoint summary/tail behavior.
- [x] Keep deterministic fallback compaction as the first implementation; do not introduce a live model summarizer yet.
- [x] Run focused e2e tests and `cargo test --test e2e`.

## Task 4: RAG 3B/3D Boundaries

- [x] Add RED tests in `tests/rag.rs`:
  - `RagPromptService` formats retrieved chunks with query metadata and a strict evidence boundary.
  - code-aware chunking keeps Rust functions/tests in coherent chunks for symbol retrieval.
- [x] Add `src/tools/rag/prompt.rs` and export it from `src/tools/rag/mod.rs`.
- [x] Extend chunking with a lightweight `CodeAwareChunker` heuristic for source files; avoid tree-sitter in this pass.
- [x] Keep default ingestion behavior compatible with existing Markdown-aware tests unless source kind is code.
- [x] Run `cargo test --features rag --test rag`.

## Task 5: Docs Cleanup

- [x] Delete `docs/GOAL.md`.
- [x] Update `docs/runtime/README.md` and `docs/runtime/implementation-status.md` so completed follow-up items are no longer listed as future gaps.
- [x] Mark `docs/04-架构与路线图.md`, `docs/05-下一步-统一执行内核.md`, and `docs/06-请求生命周期.md` as historical references pointing to `docs/runtime/`.
- [x] Keep `docs/00-README.md` as the index and make `docs/runtime/` the current runtime source of truth.
- [x] Run docs-related `rg` checks for stale references to `docs/GOAL.md` and misleading “future work” text.

## Final Verification

- [x] `cargo fmt --all --check`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test`
- [x] `cargo check --features rag --bin rove-index`
- [x] `cargo clippy --all-targets --features rag -- -D warnings`
- [x] `cargo test --features rag`
- [x] `git status --short --branch`
