# AGENTS.md

This file defines repository-wide working rules for coding agents and maintainers.
It applies to the entire repository unless a deeper directory contains its own
`AGENTS.md`. There are currently no nested overrides.

## 1. Start with the right source of truth

Use this precedence when sources disagree:

1. Reproducible behavior in current source code, tests, generated schemas, and CI.
2. `docs/runtime/` for current architecture, supported behavior, verification,
   and known gaps.
3. Root `README.md`, `MEMORY_DOCTRINE.md`, and `docs/ONBOARDING.md` for the
   maintained project overview.
4. `docs/design/` for proposed or historical target designs.
5. Older top-level design and handoff documents for design history only.

Do not silently choose one side of a code/document contradiction. Confirm the
behavior, keep the safer current contract, and update the stale current-state
document in the same change when the task includes implementation.

Every future spec must state whether it is implemented. A document marked
`Proposed / Not Implemented` is not evidence that the runtime already supports
the described types, events, configuration, or behavior.

## 2. Recommended reading order

For any non-trivial change, read:

1. [`docs/ONBOARDING.md`](docs/ONBOARDING.md)
2. [`docs/runtime/README.md`](docs/runtime/README.md)
3. The relevant current-state document under `docs/runtime/`
4. The implementation and tests for the affected subsystem
5. A future spec only when the requested work explicitly targets that design

Additional routing:

| Change area | Read first |
|---|---|
| Embedded Agent/tool loop | `core/src/`, `tests/embedding_contract.rs` |
| Persistent Engine/planning/events | `docs/runtime/react-loop.md`, `runtime/src/`, transitional `src/core/`, `tests/e2e.rs` |
| State/resume/artifacts | `docs/runtime/subsystems.md`, `runtime/src/state/`, compatibility `src/state/` |
| Providers/routing | `docs/runtime/provider-smoke.md`, `models/src/`, transitional `src/models/factory.rs` |
| Tools/safety/MCP | `docs/runtime/subsystems.md`, `src/tools/`, `tests/tool_safety.rs`, `tests/mcp.rs` |
| Memory/context | `MEMORY_DOCTRINE.md`, `runtime/src/memory/`, `runtime/src/context.rs`, `runtime/src/compaction.rs`, compatibility `src/memory/` and `src/core/context.rs` |
| RAG | `docs/runtime/integration-testing.md`, `src/tools/rag/`, RAG feature tests |
| API | `docs/runtime/implementation-guide.md`, `src/interfaces/api/`, `tests/api.rs` |
| Web | `web-ui/` tests and package scripts |
| Benchmarks | `docs/runtime/benchmark-evidence.md`, `src/bench/`, `tests/bench.rs` |

## 3. Repository map

| Path | Responsibility |
|---|---|
| `models/` | `rove-models`: normalized model protocol, providers, routing, fake provider |
| `core/` | `rove-core`: in-memory Agent loop, core events/control, tool contracts and registry |
| `runtime/` | `rove-runtime`: contracts/events, workspace, context/compaction, memory, state/artifacts/SQLite/repair/resume |
| `src/core/` | Transitional persistent Engine, planning/run coordination, durable event translation and tool turns |
| `src/models/factory.rs` | Transitional AppConfig-driven provider assembly for the root facade |
| `src/tools/` | Local tools, safety metadata, MCP proxy, optional RAG |
| `src/state/` | Transitional compatibility re-exports for `rove-runtime` state modules |
| `src/memory/` | Transitional compatibility re-exports for `rove-runtime` memory modules |
| `src/interfaces/api/` | HTTP job lifecycle and SSE |
| `src/bin/` | API, benchmark, and indexing binaries |
| `tests/` | Cross-module and integration contracts |
| `benchmarks/` | Deterministic benchmark definitions and published evidence |
| `web-ui/` | Standalone Next.js workbench |
| `scripts/` | Local development and integration runners |
| `docs/runtime/` | Current implementation source of truth |
| `docs/design/` | Proposed/target design documents |
| `docs/plans/` | Implementation plans and historical execution checklists |

## 4. Architecture invariants

Preserve these boundaries unless the task explicitly changes the architecture
and updates its tests and current documentation:

- CLI, API, Web, and benchmark paths must reuse the shared runtime instead of
  growing independent agent loops.
- Provider-specific payloads stay behind the model/provider boundary. Core
  execution consumes normalized messages, tool calls, usage, and errors.
- Tool execution goes through `ToolRegistry` and the existing safety/approval
  path. A tool description, MCP annotation, prompt, or model request cannot
  grant permission.
- Workspace paths remain bounded by the resolved workspace. Never trust
  provider- or server-supplied paths as local paths.
- Canonical stream events are the lifecycle contract shared by persistence,
  API SSE, Web, tests, and reports. Do not introduce a private parallel event
  lifecycle for one interface.
- `trace.jsonl` records event facts, `task_state.json` records resumable state,
  and `report.json` is a derived summary. Do not treat the report as the only
  durable truth.
- Completed mutations and completed plan work must not be replayed on resume.
  Unknown in-flight side effects require conservative handling.
- Memory, RAG, workspace instructions, tool output, and runtime policy are
  distinct authorities. Retrieved or generated text is not automatically a
  trusted instruction.
- Local deterministic execution must remain available without provider keys or
  network access.
- Secrets must not appear in committed configuration, normal logs, trace,
  report, API responses, screenshots, fixtures, or benchmark evidence.

## 5. Current implementation boundaries

As of 2026-07-22:

- rove is a local-first Rust runtime with CLI, API, Web, persisted run state,
  resume, provider routing, tools, layered memory, optional RAG, and
  deterministic benchmarks.
- The transitional Cargo Workspace contains the root compatibility package and
  independent `rove-models`, `rove-core`, and `rove-runtime` packages.
  `rove-models` has no local project dependency; `rove-core` depends only on
  `rove-models` and embeds with Fake Model plus a custom Tool without runtime
  state. The first `rove-runtime` slice depends only on those two packages and
  owns IDs, task/checkpoint and execution-policy contracts, Workspace/path
  safety, prompt metadata/runtime identity, approval/input provider contracts,
  canonical `StreamEvent`, context/compaction services, session/durable memory,
  and state/trace/artifact/SQLite/repair/resume services. Persistent Engine
  coordination, the session-summary post-run hook, official tools, MCP/RAG,
  and `AgentEvent -> StreamEvent` translation remain in the root facade while
  later Phase 5 slices are extracted. AppConfig-driven provider selection also
  remains there until `apps/bootstrap` is extracted.
- `docs/runtime/` describes the implemented MVP.
- MCP currently supports stdio and the existing legacy SSE path. Streamable
  HTTP, negotiated sessions, rich MCP result envelopes, and Tool Artifacts are
  proposed, not implemented.
- Versioned AgentDefinition packages, `AGENTS.md` runtime discovery,
  typed procedural knowledge, expanded execution lifecycle, and the OnCall
  reference evaluation suite are proposed, not implemented.
- This repository-level `AGENTS.md` guides maintainers and coding agents. Its
  existence does not mean the rove runtime already loads workspace
  `AGENTS.md` files into model context.

The active future design chain is:

- `docs/design/2026-07-14-agent-execution-lifecycle-design.md`
- `docs/design/2026-07-14-agent-definition-and-procedural-knowledge-design.md`
- `docs/design/2026-07-15-mcp-streamable-http-and-tool-artifacts-design.md`
- `docs/design/2026-07-15-oncall-reference-agent-evaluation-plan.md`

Use those documents to plan future implementation, not to describe current
runtime behavior.

The optional terminal-interface direction is documented separately in
`docs/design/2026-07-16-grok-build-reference-and-tui-design.md`.

## 6. Working in a dirty tree

- Inspect `git status --short` before editing.
- Existing modified or untracked files belong to the user unless the task says
  otherwise.
- Do not delete, reset, overwrite, stage, or commit unrelated work.
- Do not use destructive Git commands such as `git reset --hard` or
  `git checkout --` without explicit authorization.
- Keep changes scoped. If an existing edit overlaps the requested change,
  preserve it and report any ambiguity.
- Generated state such as `.rove/`, `target/`, `web-ui/.next/`,
  `web-ui/node_modules/`, test results, and temporary integration output must
  not be committed.

## 7. Editing rules

- Prefer small, reviewable changes with an explicit compatibility story.
- Use the repository's existing naming, error, event, and serialization
  patterns before inventing a new abstraction.
- Avoid broad mechanical rewrites unless they are necessary for the task.
- When adding serialized types or public API fields, define defaults,
  compatibility, migration, and test coverage.
- When changing events, update producers, persistence, API/OpenAPI consumers,
  Web consumers, and contract tests together.
- When changing tool safety or approval behavior, add negative tests, not only
  success tests.
- Do not add dependency crates or npm packages unless the implementation needs
  them. Explain why the existing stack is insufficient.
- Let Cargo/pnpm tooling update lockfiles only when dependencies actually
  change.
- Do not weaken lints, ignore errors, add blanket `allow` attributes, or remove
  tests to make a gate pass.

## 8. Verification

Run the smallest relevant check first, then expand in proportion to risk.

### Rust default path

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Focused examples:

```powershell
cargo test --test e2e
cargo test --test api
cargo test --test mcp
cargo test --test tool_safety
cargo test --test bench
```

### RAG feature

```powershell
cargo check --features rag --bin rove-index
cargo clippy --all-targets --features rag -- -D warnings
cargo test --features rag
```

### Web

From `web-ui/`:

```powershell
pnpm test
pnpm typecheck
pnpm build
```

Run `pnpm test:e2e` when the change affects browser-visible flows, SSE,
approval/input/cancel/resume, or the API proxy. Follow the opt-in gates in
`docs/runtime/integration-testing.md`.

### Integration and real services

- Use `scripts/integration-smoke.ps1` for the local full stack when appropriate.
- Provider, real MCP, RAG-provider, and real browser gates are opt-in. Never
  assume credentials or external services are available.
- A skipped real-service test only proves the skip path, not interoperability.
- Never point tests at production services.

### Documentation-only changes

For docs-only changes:

- inspect all relative links;
- ensure code fences are balanced;
- check heading structure and duplicate anchors;
- check trailing whitespace;
- verify current/proposed wording;
- inspect `git diff --check` and `git status --short`.

Code tests are not required solely because Markdown changed, unless the
document is parsed or asserted by tests, changes executable commands, or the
task explicitly asks for a full gate.

## 9. Documentation governance

- `docs/runtime/` describes current behavior and must be updated in the same
  implementation change that changes the contract.
- Future architecture belongs under `docs/design/` and must carry a
  visible status.
- Implementation plans belong under `docs/plans/` and must not require a
  particular external agent skill.
- Do not update `docs/runtime/implementation-status.md` or
  `docs/runtime/acceptance-matrix.md` to `Met` before code and tests exist.
- Keep examples secret-free and portable; prefer relative paths and environment
  references.
- Link to current source/test evidence for non-obvious claims.
- Preserve historical documents unless the task explicitly retires them.
- When a design decision changes, record the replacement and affected
  migration rather than silently rewriting history.

## 10. Security checklist

Before handing off a change involving tools, API, providers, state, MCP, RAG,
artifacts, or Web:

- Are input size, path, timeout, and concurrency bounded?
- Can untrusted content become instructions or permissions?
- Are secrets redacted from errors, events, config dumps, traces, reports, and
  fixtures?
- Is approval required at the correct boundary?
- Are retries safe for side effects?
- Does cancellation leave an unknown external effect?
- Can resume replay completed work?
- Are remote URLs, redirects, filenames, MIME types, and resource identifiers
  validated?
- Are API and Web behavior protected by existing auth/CORS/rate-limit rules?
- Is the failure state typed and visible instead of reported as success?

## 11. Completion and handoff

A completed change should state:

- what changed;
- which current contract or proposed design it implements;
- files intentionally not changed;
- tests/checks run and their results;
- checks not run and why;
- known risks or follow-up work;
- current `git status`, especially untracked evidence or user-owned files.

Do not claim completion from prose alone. Implementation work is complete only
when the requested behavior exists, relevant verification passes, and current
documentation agrees with the code.
