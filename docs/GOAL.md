Complete rove milestones M1 through M6, each on a separate feature branch.

## Setup

First, read all project docs:
- docs/HANDOFF-2026-05-21.md
- docs/superpowers/plans/2026-05-21-rove-m0-m2.md
- docs/01-愿景与关键决策.md
- docs/04-架构与路线图.md
- docs/07-产品定位与Workspace.md
- docs/05-下一步-统一执行内核.md
- docs/06-请求生命周期.md

Current state: M0 baseline complete on main (commit acb463c). All 6 tests pass.

## Branch Strategy

Each milestone gets its own branch, forked from the previous milestone's branch:
1. feat/m1-stateful-tools (from main)
2. feat/m2-planner (from feat/m1)
3. feat/m3-rag (from feat/m2)
4. feat/m4-mcp (from feat/m3)
5. feat/m5-http-api (from feat/m4)
6. feat/m6-web-ui (from feat/m5)

After each milestone is complete:
- Run `cargo fmt --all --check`
- Run `cargo clippy --all-targets --all-features -- -D warnings`
- Run `cargo test` (all tests must pass)
- Commit all changes
- Push the branch using: `git config --global --unset url.https://ghfast.top/https://github.com/.insteadof 2>/dev/null; git push -u origin <branch>; git config --global url."https://ghfast.top/https://github.com/".insteadOf "https://github.com/"`

## Phase 1: M1 — Stateful Tools and Resume

Follow Tasks 4 + 5 from docs/superpowers/plans/2026-05-21-rove-m0-m2.md exactly.

Key deliverables:
- ToolContext and ApprovalPolicy in src/core/types.rs
- Tool boundary module (src/core/boundary.rs) — destructive tools blocked under ApprovalPolicy::Never
- task_state.json persistence (schema_version from day 1)
- --resume latest CLI flag
- ContextManager deterministic trimming (system prompt → memory → trimmed history → current message)
- File tools (fs read/write) and shell tool with boundary checks
- Tool pipeline: schema → validate → boundary check → exec → result wrapping

TDD: write failing test → implement → verify pass → commit.

## Phase 2: M2 — Planner

Follow Task 6 + 7 from the plan doc.

Key deliverables:
- prompts/planner.md (JSON-only output)
- TaskPlan and PlanStep types in src/core/types.rs
- src/core/planner.rs — draft() method
- Engine becomes plan-aware: plan → execute step → re-plan if needed
- Plan state persisted in task_state.json
- Interrupted runs resume at correct plan step
- Final verification: fmt + clippy + test + smoke commands

## Phase 3: M3 — RAG Retriever

Based on docs/04-架构与路线图.md M3 section.

Key deliverables:
- src/tools/rag.rs with retrieve_code and retrieve_docs tools
- Ingestion pipeline: chunker + embedder + lancedb storage
- Start with OpenAI embedding API (can be swapped later)
- rove-index subcommand or binary for ingestion
- Tests for retrieval accuracy

## Phase 4: M4 — MCP Client

Based on docs/04-架构与路线图.md M4 section.

Key deliverables:
- src/tools/mcp_proxy.rs — register MCP server tools as agent tools
- MCP client implementation (JSON-RPC 2.0 over stdio/SSE)
- MCP server configuration (filesystem, github, postgres examples)
- Auto-fetch tool schemas from MCP servers
- Tests with mock MCP server

## Phase 5: M5 — HTTP API

Based on docs/04-架构与路线图.md M5 section.

Key deliverables:
- src/interfaces/api/ with axum
- POST /jobs — create async task, returns job_id
- GET /jobs/{id}/events — SSE stream of events
- GET /jobs/{id}/state — current state
- POST /jobs/{id}/cancel — cancel job
- Optional: split into Cargo workspace (rove-core, rove-cli, rove-api)
- Integration tests for API endpoints

## Phase 6: M6 — Web UI

Based on docs/04-架构与路线图.md M6 section.

Key deliverables:
- Minimal HTML/JS UI served from axum (Path B from roadmap)
- Chat interface
- Tool call visualization
- Plan progress view
- Trace timeline
- Connects to M5 API via SSE

## Constraints

- No RAG, MCP, API, Web UI before M1 is stable
- Single file max 800 lines
- Core never imports interfaces
- Prompt files in prompts/ directory, loaded at runtime
- All types use Rust enums (sum types), not strings
- Schema version on task_state.json from day 1
- Every commit must pass cargo fmt + clippy + test
- DO NOT merge any branch to main

## Verification

After completing all phases, run a final verification:
1. Check each branch passes CI
2. Verify no branch has been merged to main
3. Summarize what was implemented in each milestone
