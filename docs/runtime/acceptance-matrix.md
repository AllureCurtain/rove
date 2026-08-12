# Runtime Acceptance Matrix

This matrix maps the original M0-M6 milestone intent to the current runtime
verification surface. The historical milestone docs remain useful background;
this file is the current proof map.

| Milestone | Criterion | Current status | Verification command/test | Gap owner phase |
|---|---|---|---|---|
| M0 | Local CLI skeleton can run a model loop, emit stream events, detect Folder/Repo workspaces, and write trace/task/report artifacts. | Met | `cargo test -p rove-integration-tests --test e2e engine_produces_final_answer -- --exact`; `cargo test -p rove-integration-tests --test e2e trace_writer_records_events -- --exact`; `cargo test -p rove-integration-tests --test e2e oneshot_report_includes_workspace_and_identity_metadata -- --exact` | Closed by phases 1, 5, 12 docs upkeep |
| M1 | Core ReAct loop can call tools, enforce approval policy, persist state, and run deterministic fake-model benchmark/smoke tasks. | Met | `cargo test -p rove-integration-tests --test e2e engine_handles_tool_call -- --exact`; `cargo test -p rove-integration-tests --test tool_safety`; `cargo run -p rove-bench -- --suite benchmarks/agent-smoke.json --output-dir .rove/bench` | Closed by phases 2, 10 |
| M2 | Planner supports persisted plans, bounded multi-turn ReAct within each step, tool-result round trips before step completion, shared model/tool execution paths, append-only step results, deterministic post-step decisions, immutable replacement revisions, resume without repeating completed or unknown in-flight work, and context checkpoints. | Met | `cargo test -p rove-integration-tests --test e2e planned_step_returns_tool_result_to_model_before_completion -- --exact`; `cargo test -p rove-integration-tests --test e2e planned_step_emits_complete_step_record_before_compatibility_completion -- --exact`; `cargo test -p rove-integration-tests --test e2e replanning_retains_failed_record_and_advances_revision_identity -- --exact`; `cargo test -p rove-integration-tests --test e2e planner_resume_checkpoint_does_not_repeat_completed_steps -- --exact` | Closed. Model-on-ambiguity evaluation, independent finalization, global multidimensional budgets, and trace-tail reconciliation are implemented and covered by `cargo test -p rove-runtime --lib state::reconcile`, `cargo test -p rove-app-bootstrap --lib config::`, and `cargo test -p rove-integration-tests --test e2e execution_budget` |
| M3 | Built-in vector/RAG indexing | Removed / out of scope for local-first product | Default builds and workspace tests do not depend on lancedb/arrow | Removed |
| M4 | MCP tools register as first-class tools for CLI/API, expose annotations as tool metadata, and have byte-bounded transports plus opt-in real server smoke coverage. | Met | `cargo test -p rove-integration-tests --test mcp`; optional real server smoke: `$env:ROVE_MCP_FILESYSTEM_SMOKE="1"; cargo test -p rove-integration-tests --test mcp mcp_official_filesystem_server_smoke_when_enabled -- --exact --nocapture` | Closed by phases 3, 9; CDH G7 added 1 MiB response bounds and product catalog safety |
| M4+ | The current MCP Streamable HTTP transport negotiates protocol version and session, correlates responses by JSON-RPC id under bounded limits, pages `tools/list` cursors, enforces TLS/redirect/content-type safety, classifies session expiry, treats a committed send failure as indeterminate, and never lets a remote annotation grant local safety. | Met | `cargo test -p rove-runtime --lib tools::mcp`; `cargo test -p rove-integration-tests --test mcp_streamable_http`; focused: `cargo test -p rove-integration-tests --test mcp_streamable_http a_remote_annotation_cannot_grant_local_safety -- --exact` | Closed by full-delivery Checkpoint 3. Interoperability with a real third-party hosted MCP server remains optional and unrun |
| M4++ | A tool result carries a bounded typed envelope with distinct model/UI/finalizer/audit projections, an indeterminate outcome is never safely retryable, every MCP proxy maps rich blocks and schema/error semantics through that contract, binary content is stored as a durable content-addressed Tool Artifact under quota with ledger/event coverage, a hostile remote filename or `uri` steers nothing, and the product API serves the artifact without building a path from the requested ID. | Met | `cargo test -p rove-core --lib tool_result`; `cargo test -p rove-runtime --lib state::tool_artifacts`; `cargo test -p rove-runtime --lib tools::mcp::result_mapping`; `cargo test -p rove-api --lib product::artifacts`; `cargo test -p rove-integration-tests --test mcp`; `cargo test -p rove-integration-tests --test mcp_streamable_http`; focused: `cargo test -p rove-integration-tests --test mcp_streamable_http a_rich_mcp_result_lands_in_the_durable_artifact_store -- --exact`; `cargo test -p rove-runtime --lib state::tool_artifacts::tests::a_remote_filename_or_uri_never_steers_the_storage_path -- --exact` | Closed by full-delivery Checkpoints 4 and 6. Transient Coding Tool projections remain intentionally separate from durable Tool Artifacts |
| M4+++ | Qualified AgentDefinition packages compile into immutable run profiles; trusted root/nested workspace instructions are bounded and scope-correct; procedures are typed, selected, hydrated, and snapshot-pinned; unfinished resume uses the exact saved content; policy text cannot grant tool capability or approval. | Met | `cargo test -p rove-runtime agents:: --lib`; `cargo test -p rove-app-bootstrap -p rove-cli -p rove-api --lib`; `cargo test -p rove-integration-tests --test e2e`; `cargo test -p rove-integration-tests --test api`; `cargo test -p rove-integration-tests --test bench oncall_benchmark_v2_passes_independent_truth_and_hard_safety_gates -- --exact` | Closed by full-delivery Checkpoints 5 and 7; external-provider and holdout evidence remains optional |
| M4++++ | Streamable HTTP `listChanged` triggers bounded complete rediscovery and atomic namespace replacement; invalid refresh retains the old catalog; active runs keep pinned bindings while later runs see the new snapshot; required startup failure blocks assembly while optional failure degrades; health/circuit/runtime identity/events/API/Web stay secret-free and resume rejects real catalog drift without rejecting a timestamp-only change. | Met | `cargo test -p rove-core --lib tools`; `cargo test -p rove-runtime tools::mcp --lib`; `cargo test -p rove-runtime runtime_identity --lib`; `cargo test -p rove-integration-tests --test mcp`; `cargo test -p rove-integration-tests --test mcp_streamable_http`; `cargo test -p rove-integration-tests --test api product_mcp_crud_is_workspace_scoped_secret_free_and_used_by_product_jobs -- --exact`; `cd apps/web; pnpm test`; `cd apps/web; pnpm typecheck` | Closed by full-delivery Checkpoint 6. Live refresh is Streamable HTTP-only; real third-party MCP interoperability remains optional and unrun |
| M5 | HTTP API can create jobs, stream/replay SSE events, cancel jobs, resolve approval/input requests, persist historical state, and enforce token/CORS/rate-limit controls. | Met | `cargo test -p rove-integration-tests --test api`; focused: `cargo test -p rove-integration-tests --test api api_creates_job_streams_events_and_reports_state -- --exact`; `cargo test -p rove-integration-tests --test api api_accepts_matching_bearer_token -- --exact` | Closed by phases 3, 5 |
| M6 | Standalone Web surface consumes the API/SSE stream, supports approval/input/cancel/resume flows, token proxying, and browser E2E coverage. | Met by the default product shell and Web Complete C0-C3 on `main`. | `cd apps/web; pnpm test`; `cd apps/web; pnpm typecheck`; `cd apps/web; pnpm build`; browser checks: `cd apps/web; pnpm test:e2e`; deterministic live API: `powershell -ExecutionPolicy Bypass -File scripts/integration-smoke.ps1` | Web Complete is integrated and verified; external-provider evidence remains a separate opt-in gate |

## Productization F (Unified Conversation Control)

| Contract | Current status | Verification surface |
|---|---|---|
| One durable Send Message command with FIFO, idempotency, promotion/revoke CAS, and six delivery states | Implemented in Runtime schema v3 and ProductStore schema v12; legacy control routes remain compatibility-only | `cargo test -p rove-runtime conversation::tests::sqlite_adapter_is_fifo_idempotent_and_cas_safe --lib`; `cargo test -p rove-api unified_message --lib` |
| Canonical event, TaskState/checkpoint/report, SSE/replay, and product projection consistency | Implemented through existing trace/event path and API reflection; approvals/input/capability/cancel remain separate | Runtime event/state/report tests and API product lifecycle tests |
| Conversation-first Web transcript with bounded paging, streaming follow, stable prepend anchoring, and delivery-state actions | Implemented; Web package/browser gates need rerun in this worktree | `apps/web/chat/*`, `apps/web/state/*`, `pnpm test`, `pnpm typecheck`, `pnpm build`, `pnpm test:e2e` |
| TUI shared in-process adapter, durable queue projection, modal precedence, bounded rendering/resume | Implemented and unit-tested; Windows ConPTY/PTY smoke remains unverified | `cargo test -p rove-cli --lib` |

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
| Exact continuation from the default shell | Implemented: product turns send `product_session_id` and omit client `resume`. Since CDH G3, provider/approval/step limits are server-owned and absent from the request; a dangling provider selection is still refused locally before any optimistic turn is appended | `apps/web/state/turn-request.test.ts`; restored second-turn and missing-profile scenarios in `apps/web/tests/e2e/continuity.spec.ts` |
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

## CDH Control Completion (G1-G7)

G7 is the Settings/MCP gap-fill over the main Settings baseline, not a full alder
port. Desktop D0 is implemented on `main` through PR #30 as a Tauri delivery
shell over the shared API/Web contracts; platform evidence below is
intentionally limited to the current Windows environment.

| CDH contract | Current status | Test evidence surface |
|---|---|---|
| G1 Steer at safe points and durable follow-up queue | Implemented: server-owned drain, CAS claim, restart recovery | `product_steer_route_is_idempotent_and_applies_after_an_input_safe_point`, `product_steer_submitted_during_generation_applies_after_the_tool_safe_point`, `product_followup_after_final_is_server_owned_and_starts_one_successor` in `tests/api.rs` |
| G2 Fork at terminal boundaries with inherited read-only history | Implemented: active/corrupt sources rejected; child history independent | `product_session_fork_replays_exactly_and_keeps_child_history_independent`, `product_session_fork_rejects_incomplete_and_active_sources` in `tests/api.rs` |
| G3 Session-scoped model/reasoning with revision CAS | Implemented: mid-run immutable, applies from next run; per-run limits are session-owned | `product_session_model_changes_apply_from_the_next_run_and_keep_snapshot_history` in `tests/api.rs`; `apps/web/product-v2/QuickModelControl.test.ts` |
| G4 Usage, cost, and context occupancy | Implemented: real aggregates with explicit unavailable-when-unpriced | `product_session_usage_aggregates_report_totals_with_local_zero_cost` in `tests/api.rs` |
| G5 Files, artifacts, images, and diff | Implemented including binary stream and image format/byte/pixel validation | `product_workspace_files_list_and_content_reject_traversal`, `product_workspace_files_are_bounded_typed_and_safely_delivered`, `product_artifacts_are_hashed_session_bound_and_report_cleanup`, `product_diff_returns_canonical_tool_and_git_patches` in `tests/api.rs` |
| G6 Evidence export in JSON/HTML/Markdown, redacted | Implemented: script-free offline HTML; canary secrets scrubbed | `product_session_evidence_export_is_complete_bounded_and_redacted_in_all_formats` in `tests/api.rs` |
| G7 Settings/MCP gap-fill | Implemented: workspace-scoped catalog shared by Settings and Jobs; secret-free persistence; typed probe failures; 1 MiB transport bounds; fail-closed catalog errors | `product_mcp_crud_is_workspace_scoped_secret_free_and_used_by_product_jobs`, `product_mcp_probe_returns_typed_stdio_failures`, `product_mcp_probe_discovers_tools_over_legacy_sse`, `product_mcp_maps_corrupt_locked_and_unsafe_config_to_typed_conflicts` in `tests/api.rs`; `mcp_sse_rejects_oversized_discovery_and_json_responses`, `disabled_mcp_servers_are_never_assembled_or_environment_resolved`, `mcp_environment_resolution_rejects_invalid_and_unavailable_names` in `tests/mcp.rs`; `apps/web/settings/MCPSettings.test.tsx` and the MCP scenarios in `apps/web/tests/e2e/settings.spec.ts` |
| Desktop D0 | Implemented: embedded API router/state with bearer auth and complete shutdown drain, API-ready random loopback port, document-start WebView token injection, authenticated direct Web/SSE/binary resource transport, bounded native workspace picker wired into both shared-Web workspace forms, open/reveal commands, static Web build, and Windows MSI/NSIS/process evidence. macOS/Linux packaging, manual installation, and full interactive WebView evidence remain unverified. | `cargo test -p rove-desktop --all-targets -j 1`; `pnpm test`; `pnpm typecheck`; `pnpm build:desktop`; Windows `pnpm dlx @tauri-apps/cli@2 build --ci` and release process smoke |

### CDH hardening

| Hardening contract | Current status | Test evidence surface |
|---|---|---|
| Concurrent multi-workspace steer / follow-up / fork / model | Implemented: controls partition by session and never share ids; concurrent same-key submissions stay idempotent; concurrent CAS writes yield exactly one winner and one typed conflict, with the winner observed to alternate across runs; simultaneous release starts one successor per session; concurrent forks stay independent. Runs on a 4-worker multi-thread runtime, not await-point interleaving | `product_concurrent_multi_workspace_control_operations_stay_isolated_and_serialized` in `tests/api.rs` |
| SSE drop mid-flight | Implemented: the run is asserted live (running, input still pending, no terminal event delivered) at the moment the stream is severed; the run then completes undisturbed, and `Last-Event-ID` reconnect returns exactly the undelivered range with no gap or duplicate | `api_sse_stream_dropped_mid_flight_loses_no_events_on_reconnect` in `tests/api.rs` |
| API restart | Implemented | five `api_restart_*` / `api_startup_*` tests in `tests/api.rs` |
| Long session / deep tree / large dir / large diff | Implemented: exact per-turn ordinals over six turns; 24-level prefix resolves and still refuses traversal escape; 250-entry directory pages exactly once in stable order; 4,200 mutations cap at exactly 4,096 entries with a capped reason inside the 4 MiB budget | `product_long_session_deep_tree_large_dir_and_large_diff_stay_bounded` in `tests/api.rs` |
| Directory scan bound | Implemented: `collect_entries` takes an injectable `scan_limit`, so the cut-short path is covered without materializing 50,000 files | `directory_scan_stops_at_the_limit_and_reports_it` and `directory_scan_limit_does_not_count_skipped_secret_entries_as_results` in `apps/api/src/product/files.rs` |

Cross-cutting default gate:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Full-gate acceptance with a machine-readable verdict:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/product-acceptance.ps1
```

```bash
bash scripts/product-acceptance.sh
```

Both write `PRODUCT_ACCEPTANCE_REPORT.json`. A check never passes without a real
exit code, anything unrun carries a reason, and the verdict is `PASS` only with
zero failures and zero unrun required checks. See
[integration-testing.md](integration-testing.md) for the report contract.
