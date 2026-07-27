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
| M6 | Standalone Web surface consumes the API/SSE stream, supports approval/input/cancel/resume flows, token proxying, and browser E2E coverage. | Met by the default product shell and Web Complete C0-C3 on the stacked delivery branch. | `cd apps/web; pnpm test`; `cd apps/web; pnpm typecheck`; `cd apps/web; pnpm build`; browser checks: `cd apps/web; pnpm test:e2e`; deterministic live API: `powershell -ExecutionPolicy Bypass -File scripts/integration-smoke.ps1` | Web Complete implementation verified; stacked PR integration remains pending and external-provider evidence is a separate opt-in gate |

Evidence boundary: `shell.spec.ts`, `continuity.spec.ts`, `settings.spec.ts`,
`migration.spec.ts`, and `polish.spec.ts` use browser-boundary mocks for broad
deterministic product, state-race, fault, recovery, and visual coverage. The
gated `real-api.spec.ts` is run by `local-full` against the live Rust API. Its C3
run passed all three cases: migration before catalog boot; exact A/B session
continuity with refresh, approval, input, cancellation, Settings, and deep
routes; and one bounded `/dev/workbench` direct-run smoke. The updated provider
runner also targets the product shell and exact returned IDs, but no external
provider C3 gate has been run.

## Web Complete C0

| C0 contract | Current status | Test evidence surface |
|---|---|---|
| API-global ProductStore CRUD and safe preferences | Implemented; wired operations document actual `500`/`503` failures and never advertise foundation-era `501` | `apps/api/src/product/store/tests.rs`; product route and OpenAPI coverage in `tests/api.rs` |
| Exact product-session/runtime binding and fail-closed continuation | Implemented | `product_sessions_in_one_workspace_resume_their_own_exact_runs` and product resume/cancel tests in `tests/api.rs` |
| Canonical-event transcript projection with typed partial reasons | Implemented | transcript module tests and product transcript assertions in `tests/api.rs` |
| Strict/idempotent M1 migration and typed Web client | Implemented; C3 invokes the migration state machine before the default shell can read the product catalog | `apps/web/product/product-client.test.ts`, `apps/web/product/m1-browser-migration.test.ts`, `apps/web/shell/M1MigrationGate.test.ts`, `apps/web/tests/e2e/migration.spec.ts`, the live migration case in `real-api.spec.ts`, and migration tests in `tests/api.rs` |
| Migration preparation/apply lifecycle | Implemented: preparation has a 30-second deadline; apply is supervised, survives handler disconnect, and persists/reuses its baseline | migration lifecycle tests in `apps/api/src/product/migration.rs` and migration/store coverage in `apps/api/src/product/store/tests.rs` |
| Concurrent preference and active-session safety | Implemented: revision CAS preserves newer preferences; a source-mapped active session returns typed `product_session_active` and the Web keeps the exact retry payload | preference/active-session migration tests in `apps/api/src/product/store/tests.rs` and `apps/web/product/m1-browser-migration.test.ts` |
| Runtime binding commit safety | Implemented: canonical sorted runtime reservations, workspace containment, `SQLITE_OPEN_NOFOLLOW`, and symlink-parent rejection | external commit-guard tests in `runtime/src/state/index.rs` and migration tests in `tests/api.rs` |
| Product job-start shutdown ownership | Implemented: owned start tasks drain before job supervisors and handles | lifecycle tests in `apps/api/src/lib.rs` |

## Web Complete C1

C1 was implemented with focused and mock-backed browser evidence. C3 now adds a
live-API cross-session continuation and refresh scenario without reclassifying
the broader C1 race/fault-injection cases as live evidence.

| C1 contract | Current status | Test evidence surface |
|---|---|---|
| API-authoritative boot catalog, preferences, and provider profiles | Implemented in the default `ProductApp` | `apps/web/state/product-catalog.test.ts`; `apps/web/state/server-product-state.ts`; provider persistence scenario in `apps/web/tests/e2e/continuity.spec.ts` |
| Durable workspace/session/Settings routes | Implemented with explicit invalid/mismatched route failures | `apps/web/state/product-route.test.ts`; route landing and no-wrong-session-flash scenarios in `apps/web/tests/e2e/continuity.spec.ts` |
| Canonical transcript restore | Implemented with complete, explicit partial, error, retry, and session-switch race handling | `apps/web/state/transcript-projection.test.ts`; restore scenarios in `apps/web/tests/e2e/continuity.spec.ts` |
| Exact continuation from the default shell | Implemented: product turns send `product_session_id` and omit client `resume` | `apps/web/state/turn-request.test.ts`; restored second-turn scenario in `apps/web/tests/e2e/continuity.spec.ts` |
| Focused reattachment and background status | Implemented: one focused live observation plus bounded background catalog polling | focused reattachment and background attention scenarios in `apps/web/tests/e2e/continuity.spec.ts` |
| Ambiguous `POST /jobs` response | Implemented: bounded binding reconciliation, no automatic duplicate submission, transcript fallback/explicit uncertainty | disconnect plus delayed-binding scenario in `apps/web/tests/e2e/continuity.spec.ts` |

## Web Complete C2

| C2 contract | Current status | Test evidence surface |
|---|---|---|
| Nine usable Settings sections | Implemented; no placeholder-only route remains | section and mobile scenarios in `apps/web/tests/e2e/settings.spec.ts` |
| Provider CRUD, test, models, and selection | Implemented through the API store without raw keys | client/unit tests plus provider scenarios in `continuity.spec.ts`, `shell.spec.ts`, and `settings.spec.ts` |
| Approval defaults and execution limits | Implemented with preference revision CAS; the API default is used when a turn omits approval | `product_default_approval_is_honored_and_explicit_approval_wins` in `tests/api.rs`; state and browser policy tests |
| Workspace/session management | Implemented for pin/remove, rename/delete, and bounded safe catalog export | catalog model tests and `settings.spec.ts` |
| Memory and runtime health | Implemented with bounded list/read/delete and redacted health contracts | API/memory tests, settings client/model tests, and `settings.spec.ts` |
| Critical keyboard shortcuts | Implemented for composer focus, new session, Settings, and Inspector | keyboard matcher tests and `settings.spec.ts` |

## Web Complete C3

| C3 contract | Current status | Test evidence surface |
|---|---|---|
| Migration before catalog authority | Implemented: only `not_needed` or verified `complete` mounts server product state; uncertain or invalid state remains fail closed | gate unit tests, `migration.spec.ts`, and the live migration case in `real-api.spec.ts` |
| Exact live product-session continuity | Implemented and verified across interleaved A/B sessions and refresh; no workspace-global latest guess is used | product-shell lifecycle case in `real-api.spec.ts`, run by `integration-smoke.ps1` |
| Live interactions and durable routes | Approval, input, cancellation, Settings, provider persistence, Memory/health surfaces, and deep routes are exercised against the live Rust API | product-shell lifecycle case in `real-api.spec.ts` |
| Final product polish | Responsive bounds, visible focus, keyboard behavior, reduced motion, theme/state presentation, and migration recovery are implemented | focused Web tests plus `polish.spec.ts` and `migration.spec.ts` |
| Advanced escape hatch | Retained as one bounded direct-run smoke, not a second primary product entry | optional workbench case in `real-api.spec.ts`, enabled by `integration-smoke.ps1` |
| Provider browser flow | Updated to create API-backed product state and verify exact browser-returned job/run IDs in reports and transcripts | `scripts/provider-integration.ps1`; external-provider execution was not run for C3 |

Cross-cutting default gate:

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```
