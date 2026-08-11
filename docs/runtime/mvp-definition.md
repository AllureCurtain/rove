# MVP Definition

Status: MVP reached for the local-first single-user runtime.
Date: 2026-05-30
Last interface update: 2026-08-06 (CDH G1-G7 control, evidence, and Settings completion).

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
- Standalone Web M1 product shell at `/` for Workspace/Session navigation,
  chat/SSE, inline tool approval/input/cancel, Inspector state, and provider
  controls. The advanced `/dev/workbench` retains direct resume/history/report
  workflows from the historical M6 surface.
- Web Complete C0 product-control foundation:
  API-global SQLite workspace/session/profile/preferences state, exact
  product-session/runtime bindings, canonical-event transcript reads,
  strict/idempotent M1 browser migration, and typed Web client/migration
  modules.
- Web Complete C1 default-shell integration: API-authoritative catalog,
  preferences, and provider profiles; durable workspace/session/Settings
  routes; canonical transcript restore with explicit partial/error/retry
  states; exact `product_session_id` turns; focused live-job reattachment;
  background status polling; and bounded no-duplicate reconciliation after an
  ambiguous job-start response.
- Web Complete C2 Settings completion: revision-safe default approval
  preferences honored by later jobs; bounded memory and runtime-health APIs;
  provider profile create/read/update/delete; durable workspace/session
  management; safe catalog export; and four wired keyboard shortcuts across
  all nine Settings sections.
- Web Complete C3 completion: replay-safe M1 migration gates catalog boot and
  exposes explicit fail-closed recovery; legacy product routes remap through the
  verified acknowledgement; responsive, focus, keyboard, reduced-motion, and
  product-state polish is complete; and the default shell has deterministic
  live-API coverage for migration, exact A/B continuation, refresh,
  approval/input/cancel, Settings, and deep routes. One bounded advanced
  `/dev/workbench` smoke remains available.
- CDH G1-G7 completion: API-authoritative Steer/Follow-up, terminal-boundary
  Fork with inherited read-only lineage, revision-safe session model/reasoning/
  approval/step-limit configuration with immutable run snapshots, explicit
  usage/context/cost states, bounded workspace files/artifacts/images/diff,
  redacted JSON/HTML/Markdown evidence export, and workspace-scoped MCP catalog
  management shared by Settings and runtime assembly.
- One Runtime-neutral Core Agent kernel shared by embedded, unplanned, and planned-step execution, with Runtime-owned context, tool safety, and durable events.
- Local state under `.rove/` with trace, task state, report, and SQLite index.
- Folder, Repo, and Task workspaces.
- Built-in filesystem, shell, memory, request-input, and MCP tools. No built-in vector RAG.
- Provider abstraction for OpenAI, OpenAI Responses, Anthropic, Ollama, and fake providers.
- Deterministic no-network benchmarks and default test coverage.

Web Complete C0-C3 is implemented and verified on `main` through PRs #24–#26;
CDH G1-G7 are implemented through PR #29 at `f9e88a7`.
Broad deterministic race, failure, migration-recovery, and visual scenarios
remain browser-boundary mock evidence; `local-full` separately passed three
live-API browser cases covering migration, the default product lifecycle, and a
bounded advanced smoke after merge. No
external-provider C3 browser gate has been run.

## Out of scope

- Browser/Desktop workspace implementations.
- Multi-user identity, login, hosted billing, distributed rate limiting, and SaaS deployment controls.
- Full shell sandboxing beyond current local policy, timeout, output, denylist, and approval controls.
- Built-in vector or provider-backed RAG retrieval; future semantic retrieval
  requires a separate optional-external-service design.
- Long-running human-in-the-loop reconstruction after process restart.
- One shared Core/Runtime Agent kernel, Project Trust activation, a shared
  Execution Environment, Coding Tool V2, and product Subagents.

## Golden paths

1. CLI smoke:

   ```powershell
   cargo run -p rove-cli -- --model fake "echo hello from rove"
   ```

2. API and Web smoke:

   ```powershell
   cargo run -p rove-api
   cd apps/web
   pnpm dev
   ```

3. Deterministic benchmark:

   ```powershell
   cargo run -p rove-bench -- --suite benchmarks/agent-smoke.json --output-dir .rove/bench
   ```

4. Resume state:

   ```powershell
   cargo run -p rove-cli -- --model fake "inspect this workspace"
   cargo run -p rove-cli -- sessions
   cargo run -p rove-cli -- --resume latest --model fake "continue"
   ```

## Required verification baseline

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd apps/web
pnpm test
pnpm typecheck
pnpm build
```

Optional Unix TUI PTY verification also remains separate from the default gate:

```powershell
python scripts/tui-pty-smoke.py --run
```

On Windows this command exits `77` with `status: "skipped"`; no native ConPTY
automation result is currently available.
