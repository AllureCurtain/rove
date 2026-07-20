# Runtime Acceptance Matrix

This matrix maps the original M0-M6 milestone intent to the current runtime
verification surface. The historical milestone docs remain useful background;
this file is the current proof map.

| Milestone | Criterion | Current status | Verification command/test | Gap owner phase |
|---|---|---|---|---|
| M0 | Local CLI skeleton can run a model loop, emit stream events, detect Folder/Repo workspaces, and write trace/task/report artifacts. | Met | `cargo test --test e2e engine_produces_final_answer -- --exact`; `cargo test --test e2e trace_writer_records_events -- --exact`; `cargo test --test e2e oneshot_report_includes_workspace_and_identity_metadata -- --exact` | Closed by phases 1, 5, 12 docs upkeep |
| M1 | Core ReAct loop can call tools, enforce approval policy, persist state, and run deterministic fake-model benchmark/smoke tasks. | Met | `cargo test --test e2e engine_handles_tool_call -- --exact`; `cargo test --test tool_safety`; `cargo run --bin rove-bench -- --suite benchmarks/agent-smoke.json --output-dir .rove/bench` | Closed by phases 2, 10 |
| M2 | Planner supports persisted plans, bounded multi-turn ReAct within each step, tool-result round trips before step completion, replanning, shared model/tool execution paths, resume without repeating completed steps, and context checkpoints. | Met | `cargo test --test e2e planned_step_returns_tool_result_to_model_before_completion -- --exact`; `cargo test --test e2e planned_step_model_turn_budget_exhaustion_is_explicit -- --exact`; `cargo test --test e2e planner_replans_after_step_failure -- --exact`; `cargo test --test e2e planner_resume_checkpoint_does_not_repeat_completed_steps -- --exact` | Closed for the current compatibility contract; ledger/evaluator/finalizer remain future lifecycle phases |
| M3 | RAG tools and indexing are feature-gated, deterministic by default for local verification, configurable for provider embeddings, and write artifacts under configured state paths. | Met | `cargo check --features rag --bin rove-index`; `cargo test --features rag --test cli_index`; `cargo test --features rag --test rag` | Closed by phase 6 |
| M4 | MCP tools register as first-class tools for CLI/API, expose annotations as tool metadata, and have bounded stdio transport plus opt-in real server smoke coverage. | Met | `cargo test --test mcp`; optional real server smoke: `$env:ROVE_MCP_FILESYSTEM_SMOKE="1"; cargo test --test mcp mcp_official_filesystem_server_smoke_when_enabled -- --exact --nocapture` | Closed by phases 3, 9 |
| M5 | HTTP API can create jobs, stream/replay SSE events, cancel jobs, resolve approval/input requests, persist historical state, and enforce token/CORS/rate-limit controls. | Met | `cargo test --test api`; focused: `cargo test --test api api_creates_job_streams_events_and_reports_state -- --exact`; `cargo test --test api api_accepts_matching_bearer_token -- --exact` | Closed by phases 3, 5 |
| M6 | Standalone Web workbench consumes the API/SSE stream, supports approval/input/cancel/resume flows, token proxying, and browser E2E coverage. | Met | `cd web-ui; pnpm test`; `cd web-ui; pnpm typecheck`; `cd web-ui; pnpm build`; optional browser check: `cd web-ui; pnpm test:e2e` | Closed by phases 3, 8 |

Cross-cutting default gate:

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```
