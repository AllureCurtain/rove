# rove

A local-first coding agent: one persistent Rust runtime powering Desktop, Web, CLI/TUI, HTTP API, and deterministic evaluation. Single user, running on their own machine.

## Tech stack

- Runtime and backend: Rust stable (Cargo workspace, resolver 3), Tokio, Axum + utoipa, rusqlite (bundled)
- Terminal: clap, ratatui + crossterm, rustyline
- Web: Next.js 16 + React 19 + TypeScript, pnpm 10, Node 22, vitest + Playwright
- Desktop: Tauri 2, embedding `rove-api` and the Web static build
- Models: OpenAI Chat / Responses, Anthropic, Ollama, Fake, external process adapters

## Layout

```
models/          rove-models: model protocols, provider adapters, routing, Fake model (no local deps)
core/            rove-core: in-memory Agent kernel, Tool contract and registry
runtime/         rove-runtime: Engine, planning, state and recovery, tools/MCP, memory, context; conventions in runtime/AGENTS.md
protocol/        rove-protocol: lowest-level shared types
tools-text/      rove-tools-text: text tool helpers
apps/bootstrap/  rove-app-bootstrap: configuration, Project Trust, Engine assembly
apps/cli/        rove-cli: the `rove` binary (TUI / REPL / exec / admin commands)
apps/product-store/  rove-product-store: product contract types and ProductStore (SQLite), no HTTP
apps/api/        rove-api: HTTP/SSE/OpenAPI, conventions in apps/api/AGENTS.md
apps/bench/      rove-bench: deterministic benchmarks
apps/desktop/    rove-desktop: Tauri host
apps/web/        Next.js product UI, conventions in apps/web/AGENTS.md
tests/           rove-integration-tests: cross-crate contract tests
benchmarks/      benchmark definitions and evidence
scripts/         development, integration, and acceptance scripts
docs/            project documentation
```

Dependencies flow one way only: `rove-models <- rove-core <- rove-runtime <- rove-app-bootstrap <- {rove-cli, rove-api, rove-bench}`; `rove-product-store <- rove-api <- rove-desktop`; `rove-protocol` is the lowest-level type shared by core/runtime; `rove-models` has no local dependencies. Guarded by `tests/workspace_architecture.rs`.

## Common commands

Full command list in docs/development.md.

```bash
cargo run -p rove-cli -- --model fake                 # launch TUI with no API key
cargo check -p <crate>                                 # check only the crate you changed
cargo test -p <crate>                                  # targeted tests
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
cd apps/web && pnpm test && pnpm typecheck             # Web
```

## Hard rules

- CLI, API, Web, and benchmarks share the same runtime Engine; never create a second agent loop.
- Provider-specific formats stay inside `rove-models`; the execution layer only consumes normalized messages, tool calls, usage, and errors.
- Tool execution must go through `ToolRegistry` and the existing safety and approval paths. Tool descriptions, MCP annotations, prompts, and model output cannot grant permissions.
- Workspace paths must stay inside the resolved workspace; never trust paths from a provider or server.
- `StreamEvent` is the only lifecycle contract; persistence, SSE, Web, and tests all share it. Never create a parallel event set for one surface.
- `trace.jsonl` is the event record, `task_state.json` is recoverable state, `report.json` is only a derived summary.
- Recovery must not replay completed changes or completed plan steps; unknown in-flight side effects are handled conservatively.
- With no provider key and no network, local deterministic execution must work (`--model fake`).
- Secrets must never appear in committed config, logs, traces, reports, API responses, screenshots, fixtures, or benchmark evidence.
- API fields are authoritative in the runtime-generated OpenAPI (`/api/openapi.json`, utoipa annotations). Change types and annotations first, then the frontend. The checked-in `apps/api/openapi.json` snapshot is kept in sync by tests; regenerate it with `ROVE_UPDATE_OPENAPI=1` after API changes.
- ProductStore and runtime StateIndex schemas may only change through their respective schema migrations (`apps/product-store/src/store/schema.rs`, `runtime/src/state/index.rs`); conventions in apps/api/AGENTS.md and runtime/AGENTS.md.
- Read DESIGN.md before touching styles; design tokens only.
- Read docs/decisions.md before making a technology choice or overturning an existing one.
- A new dependency must explain why existing ones are insufficient; do not relax lints, add crate-level `allow`, or delete tests to make checks pass.
- All changes go through branches and PRs under `.worktrees/`; never commit directly on `main`. See docs/development.md for the process.

## Doc index

| When | Read |
| --- | --- |
| Starting any task | TODO.md |
| Module boundaries, data flow, state directories | docs/architecture.md |
| API work | docs/api.md, `apps/api/openapi.json` (snapshot), and runtime `/api/openapi.json` |
| UI work | DESIGN.md |
| Web information architecture, default visibility, interactions | docs/web-console-design.md |
| Environment, commands, tests, release, Git process | docs/development.md |
| Changing the technical approach | docs/decisions.md |
| User-visible feature changes | docs/user-guide.md |
| Structural refactor in progress | docs/refactor-plan.md |

## Doc update rules

Update documentation in the same commit as the code:

- Starting or finishing a task: update TODO.md
- Module boundaries, data flow, or state layout changed: update docs/architecture.md
- A technical decision made: append an entry to docs/decisions.md
- New command or environment variable: update docs/development.md and `.env.example` (if applicable)
- User-visible feature change: update docs/user-guide.md
- New design rule or token: update DESIGN.md

This file is also loaded by rove itself as workspace root instructions; loading it does not grant any tool permissions.
