# MVP Definition

Status: MVP reached for the local-first single-user runtime.
Date: 2026-05-30

## Definition

The rove MVP is a local-first agent runtime that can run from CLI, API, and Web, stream observable events, call bounded tools, persist readable artifacts, resume from saved state, and run deterministic verification without network credentials.

This MVP is not a SaaS product, browser automation runtime, desktop automation runtime, or multi-user hosted service.

## Included

- CLI one-shot runs and line-oriented REPL.
- HTTP API job lifecycle with SSE, cancel, approval, input, resume, and persisted replay.
- Standalone Web workbench for submitting jobs, streaming events, approving tools, answering input requests, cancelling runs, resuming latest state, and viewing historical run reports.
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
