# MVP Definition

Status: MVP reached for the local-first single-user runtime.
Date: 2026-05-30
Last interface update: 2026-07-19 (bounded full-screen TUI navigation, timeline, and terminal hardening).

## Definition

The rove MVP is a local-first agent runtime that can run from CLI, API, and Web, stream observable events, call bounded tools, persist readable artifacts, resume from saved state, and run deterministic verification without network credentials.

This MVP is not a SaaS product, browser automation runtime, desktop automation runtime, or multi-user hosted service.

## Included

- CLI one-shot runs, line-oriented REPL, and the optional full-screen TUI with
  bounded approval/input interaction, session navigation/resume selection,
  tool-detail/help overlays, chronological visible timeline, resize handling,
  and terminal restoration (`rove tui --model fake`). Non-Windows terminals
  with keyboard enhancement use direct `Y`/`Enter` actions; Windows uses the
  non-text `F8` confirmation/submission path. Unsupported terminals retain the
  basic TUI and fail closed for live interaction. Unix PTY smoke is opt-in;
  Windows ConPTY automation is not included and reports a typed skip.
- HTTP API job lifecycle with SSE, cancel, approval, input, resume, and persisted replay.
- Standalone Web workbench for submitting jobs, streaming events, approving tools, answering input requests, cancelling runs, resuming latest state, and viewing historical run reports.
- Core engine with planned and unplanned loops sharing model turns, tool turns, context checkpoints, and history writeback.
- Local state under `.rove/` with trace, task state, report, and SQLite index.
- Folder, Repo, and Task workspaces.
- Built-in filesystem, shell, memory, request-input, and MCP tools. No built-in vector RAG.
- Provider abstraction for OpenAI, OpenAI Responses, Anthropic, Ollama, and fake providers.
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
   cd apps/web
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
cd apps/web
pnpm test
pnpm typecheck
pnpm build
```

Optional RAG verification remains separate:

```powershell


```

Optional Unix TUI PTY verification also remains separate from the default gate:

```powershell
python scripts/tui-pty-smoke.py --run
```

On Windows this command exits `77` with `status: "skipped"`; no native ConPTY
automation result is currently available.
