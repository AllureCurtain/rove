# Runtime Acceptance Matrix

This matrix maps the original M0-M6 milestone intent to the current runtime
verification surface. The historical milestone docs remain useful background;
this file is the current proof map.

| Milestone | Criterion | Current status | Verification command/test | Gap owner phase |
|---|---|---|---|---|
| M0 | Local CLI skeleton can run a model loop, emit stream events, detect Folder/Repo workspaces, and write trace/task/report artifacts. | Met | `cargo test --test e2e engine_produces_final_answer -- --exact`; `cargo test --test e2e trace_writer_records_events -- --exact`; `cargo test --test e2e oneshot_report_includes_workspace_and_identity_metadata -- --exact` | Closed by phases 1, 5, 12 docs upkeep |
| M1 | Core ReAct loop can call tools, enforce approval policy, persist state, and run deterministic fake-model benchmark/smoke tasks. | Met | `cargo test --test e2e engine_handles_tool_call -- --exact`; `cargo test --test tool_safety`; `cargo run --bin rove-bench -- --suite benchmarks/agent-smoke.json --output-dir .rove/bench` | Closed by phases 2, 10 |
| M2 | Planner supports persisted plans, bounded multi-turn ReAct within each step, tool-result round trips before step completion, shared model/tool execution paths, append-only step results, deterministic post-step decisions, immutable replacement revisions, resume without repeating completed or unknown in-flight work, and context checkpoints. | Met | `cargo test --test e2e planned_step_returns_tool_result_to_model_before_completion -- --exact`; `cargo test --test e2e planned_step_emits_complete_step_record_before_compatibility_completion -- --exact`; `cargo test --test e2e replanning_retains_failed_record_and_advances_revision_identity -- --exact`; `cargo test --test e2e planner_resume_checkpoint_does_not_repeat_completed_steps -- --exact` | Closed for the implemented rule-first lifecycle slice; model-on-ambiguity evaluation, independent finalization, global multidimensional budgets, and trace-tail reconciliation remain future work |
| M3 | Built-in vector/RAG indexing | Removed / out of scope for local-first product | Default builds and workspace tests do not depend on lancedb/arrow | Removed |
| M4 | MCP tools register as first-class tools for CLI/API, expose annotations as tool metadata, and have bounded stdio transport plus opt-in real server smoke coverage. | Met | `cargo test --test mcp`; optional real server smoke: `$env:ROVE_MCP_FILESYSTEM_SMOKE="1"; cargo test --test mcp mcp_official_filesystem_server_smoke_when_enabled -- --exact --nocapture` | Closed by phases 3, 9 |
| M5 | HTTP API can create jobs, stream/replay SSE events, cancel jobs, resolve approval/input requests, persist historical state, and enforce token/CORS/rate-limit controls. | Met | `cargo test --test api`; focused: `cargo test --test api api_creates_job_streams_events_and_reports_state -- --exact`; `cargo test --test api api_accepts_matching_bearer_token -- --exact` | Closed by phases 3, 5 |
| M6 | Standalone Web surface consumes the API/SSE stream, supports approval/input/cancel/resume flows, token proxying, and browser E2E coverage. | Met for the historical advanced Workbench; the default M1 product shell and C0 API/client foundation are implemented, with C1–C3 UI gaps still open | `cd apps/web; pnpm test`; `cd apps/web; pnpm typecheck`; `cd apps/web; pnpm build`; optional browser check: `cd apps/web; pnpm test:e2e` | Historical M6 closed by phases 3, 8; full live-API product-shell acceptance belongs to Web Complete C3 |

Evidence boundary: `apps/web/tests/e2e/shell.spec.ts` covers the default `/`
product shell with browser-boundary mocks. The gated real-API suite
`apps/web/tests/e2e/real-api.spec.ts` opens `/dev/workbench`, so its three tests
prove the advanced Workbench/API lifecycle rather than C1 refresh continuity or
full live-API acceptance of the product shell.

## Web Complete C0

| C0 contract | Current status | Test evidence surface |
|---|---|---|
| API-global ProductStore CRUD and safe preferences | Implemented; wired operations document actual `500`/`503` failures and never advertise foundation-era `501` | `apps/api/src/product/store/tests.rs`; product route and OpenAPI coverage in `tests/api.rs` |
| Exact product-session/runtime binding and fail-closed continuation | Implemented | `product_sessions_in_one_workspace_resume_their_own_exact_runs` and product resume/cancel tests in `tests/api.rs` |
| Canonical-event transcript projection with typed partial reasons | Implemented | transcript module tests and product transcript assertions in `tests/api.rs` |
| Strict/idempotent M1 migration and typed Web client | Implemented, not wired into the default shell | `apps/web/product/product-client.test.ts`, `apps/web/product/m1-browser-migration.test.ts`, and migration tests in `tests/api.rs` |
| Migration preparation/apply lifecycle | Implemented: preparation has a 30-second deadline; apply is supervised, survives handler disconnect, and persists/reuses its baseline | migration lifecycle tests in `apps/api/src/product/migration.rs` and migration/store coverage in `apps/api/src/product/store/tests.rs` |
| Concurrent preference and active-session safety | Implemented: revision CAS preserves newer preferences; a source-mapped active session returns typed `product_session_active` and the Web keeps the exact retry payload | preference/active-session migration tests in `apps/api/src/product/store/tests.rs` and `apps/web/product/m1-browser-migration.test.ts` |
| Runtime binding commit safety | Implemented: canonical sorted runtime reservations, workspace containment, `SQLITE_OPEN_NOFOLLOW`, and symlink-parent rejection | external commit-guard tests in `runtime/src/state/index.rs` and migration tests in `tests/api.rs` |
| Product job-start shutdown ownership | Implemented: owned start tasks drain before job supervisors and handles | lifecycle tests in `apps/api/src/lib.rs` |

Cross-cutting default gate:

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```
