# Provider Layer Redesign Progress

> Status: **Complete — Stages 0–9 DONE; channel UX follow-up applied**
>
> Started: 2026-07-23
>
> Last updated: 2026-07-24
>
> Design:
> [`../design/2026-07-23-provider-layer-redesign-design.md`](../design/2026-07-23-provider-layer-redesign-design.md)

## 1. Working location

This work is isolated from the original working tree.

| Item | Value |
|---|---|
| Worktree | `D:\Study\project\agent\rove-provider-layer` |
| Branch | `feature/provider-layer-redesign` |
| Baseline | `f1de256` (`chore(rag): remove built-in LanceDB vector RAG`) |
| Source branch | `migration/modular-workspace` |
| Original tree | `D:\Study\project\agent\rove` (user-owned dirty tree; do not edit or clean) |

The source branch was selected because the Provider work targets the completed
modular Workspace layout. It is intentionally not based on `main`, which does
not contain that migration.

## 2. Status semantics

| Marker | Meaning |
|---|---|
| `DONE` | Code/docs and required stage verification completed. |
| `ACTIVE` | Current stage; safe resume point is recorded below. |
| `PENDING` | Not started. |
| `BLOCKED` | Cannot continue without a recorded decision or external state. |

A stage moves to `DONE` only after its acceptance checks pass and the evidence
is appended to the progress log. Partial code is never marked complete.

## 3. Stage ledger

| Stage | Status | Deliverable | Required evidence |
|---|---|---|---|
| 0. Recovery and contract | DONE | Independent worktree, current-code audit, accepted design, and this ledger. | Worktree/status evidence; design links and Markdown checks. |
| 1. Provider foundation | DONE | Open protocol ID, protocol/decoder contracts, duplicate-safe registry, and byte-safe SSE/JSONL framing. No production client switch. | `cargo test -p rove-models`; fmt; clippy for `rove-models`. |
| 2. Shared HTTP transport | DONE | Bounded URL/header/auth/error handling and a single framing/decoder driver. | Deterministic mock HTTP tests, timeout/error/security negatives. |
| 3. OpenAI Chat migration | DONE | OpenAI Chat request strategy and decoder behind `ProviderClient`; legacy parity retained. | Request, fragmented stream, tool, usage, error, and identity parity tests. |
| 4. OpenAI Responses migration | DONE | Responses strategy/decoder and cache options behind the shared client. | Existing Responses tests plus transport parity tests. |
| 5. Anthropic and Ollama migration | DONE | Anthropic SSE and Ollama JSONL strategies behind the shared client. | Native request/tool/usage/error parity tests for both protocols. |
| 6. Profiles, secrets, and assembly | DONE | Named profiles, env/file secret refs, legacy conversion, registry-driven bootstrap/fallback assembly. | Config precedence/migration/redaction tests and routing identity tests. |
| 7. API inventory and Web convergence | DONE | Shared profile contract and capability-driven inventory across API/Web. | API/OpenAPI/Web unit tests and relevant browser E2E. |
| 8. External adapter v1 | DONE | Opt-in bounded process adapter for unsupported wire formats without rebuilding Rove. | Fixture adapter happy path, malformed stream, timeout, cancellation, cleanup, secret, and permission negatives. |
| 9. Cleanup and release evidence | DONE | Retire migrated clients/compatibility code as approved; current docs and integration evidence. | Workspace fmt/clippy/tests, Web gates, deterministic full integration, opt-in real-provider results if credentials exist. |

## 4. Compatibility checkpoints

These checks apply at every code stage:

- Existing `ModelClient` and `ModelEvent` consumers compile unchanged.
- Fake provider works without network or credentials.
- Routing does not retry or fall back after committed output/tool use.
- Provider-specific tool-call IDs and history remain intact.
- No secret is added to `Debug`, `Display`, errors, events, reports, API data,
  screenshots, or fixtures.
- Unknown protocol/config values fail explicitly; no silent OpenAI fallback.
- Current runtime docs change only when runtime behavior changes.

## 5. Resume protocol

After any interruption:

1. Open this file and locate the only `ACTIVE` stage.
2. Run:

   ```powershell
   git -C D:\Study\project\agent\rove-provider-layer status --short
   git -C D:\Study\project\agent\rove-provider-layer branch --show-current
   git -C D:\Study\project\agent\rove-provider-layer rev-parse --short HEAD
   ```

3. Read the latest progress-log entry and inspect its listed changed files.
4. Preserve all existing changes; do not reset or clean to resume.
5. Re-run the smallest recorded focused check before adding new behavior.
6. When a stage finishes, update its status and append exact commands/results
   before starting the next stage.

## 6. Current safe resume point

All stages 0-9 are DONE. The Provider Layer redesign is implemented and
verified on `feature/provider-layer-redesign` in the sibling worktree
`D:\Study\project\agent\rove-provider-layer`. Next human step is review and
commit/PR from that branch without touching the original dirty `rove` tree.
Legacy native HTTP client modules remain only as parity-test references; a
future follow-up may delete them after a separate compatibility-window review.

## 7. Progress log

### 2026-07-23 - Stage 0 complete

Completed:

- Recovered the previous Provider design context from the current handoff and
  uncommitted design material. The originally supplied external session UUID
  was not present as a local Codex rollout/index entry.
- Confirmed the current implementation still uses four native client modules,
  a closed `ProviderKind`/`ProviderSpec` factory, and API-side provider-name
  inventory switches.
- Created the sibling worktree and branch listed in section 1 without changing
  `main` or the original dirty tree.
- Clarified that a Rust in-process registry alone cannot provide no-rebuild
  support for arbitrary wire formats; recorded the external adapter phase.
- Recorded accepted decisions for named profiles, explicit protocol selection,
  bounded legacy compatibility, and initial env/file secret sources.

Verification:

- `git status --short` in the new worktree was clean before documentation was
  added.
- `git worktree list` showed the original and Provider worktrees at `f1de256`
  on separate branches.
- Baseline `cargo test -p rove-models` began from a cold build but exceeded the
  initial 120-second command timeout before producing a test result. It must be
  rerun with a longer timeout during Stage 1 verification; this timeout is not
  recorded as a pass or failure.

Files introduced:

- `docs/design/2026-07-23-provider-layer-redesign-design.md`
- `docs/plans/2026-07-23-provider-layer-redesign.md`

Next:

- Implement Stage 1 foundation and focused tests, then replace this resume
  point and mark Stage 1 only after all required checks pass.

### 2026-07-23 - Stage 1 complete

Completed:

- Added validated, serde-safe `WireProtocolId` with stable built-in IDs and
  support for namespaced application IDs.
- Added `WireProtocol`, per-request `StreamDecoder`, `WireRequest`, framing,
  and authentication-style contracts without switching production clients.
- Added `WireProtocolRegistry`; duplicate IDs fail without replacing the
  existing strategy, and unknown IDs report a sorted available-ID list.
- Added bounded byte-oriented SSE and JSONL framing. It preserves UTF-8 split
  across network chunks, handles CRLF/comments/multi-line SSE data, flushes a
  final unterminated provider frame, and rejects invalid UTF-8 or oversized
  lines/frames.
- Kept all existing OpenAI, Responses, Anthropic, Ollama, fake, routing, and
  health behavior unchanged.

Verification:

- `cargo test -p rove-models` - passed, 75 tests, 0 failures. This includes 13
  new Provider foundation/framing tests.
- `cargo fmt --all --check` - passed.
- `cargo clippy -p rove-models --all-targets -- -D warnings` - passed.
- `git diff --check` - passed.

Files introduced or changed:

- `models/src/lib.rs`
- `models/src/provider/mod.rs`
- `models/src/provider/id.rs`
- `models/src/provider/wire.rs`
- `models/src/provider/registry.rs`
- `models/src/provider/framing.rs`
- `docs/design/2026-07-23-provider-layer-redesign-design.md`
- `docs/plans/2026-07-23-provider-layer-redesign.md`

Next:

- Implement Stage 2 resolved auth and shared bounded HTTP transport with local
  mock-server tests. Do not migrate a native provider until Stage 2 is green.

### 2026-07-23 - Stage 2 complete

Completed:

- Added `Redacted`, `ResolvedAuth`, and `ResolvedHeader`; secret-bearing values
  never expose contents through `Debug` or `Display`, and invalid header values
  fail before any request.
- Added explicit, bounded Transport configuration for connect/request/idle
  timeouts, error bodies, framing, and redirect behavior. Redirects default to
  disabled and configured limits have hard upper bounds.
- Added endpoint/path validation for HTTP(S), credentials, hosts, query and
  fragment handling, parent paths, and request-path length.
- Reserved authentication and transport-managed headers, rejecting collisions
  rather than allowing protocol/profile data to override them.
- Added one shared request/response driver that injects auth after protocol
  construction, reads bounded/redacted HTTP error bodies, frames SSE/JSONL,
  drives per-stream decoders, enforces idle timeouts, and stops on `Done`.
- Added non-retryable `ModelError::InvalidConfiguration` and routing coverage
  proving configuration failures are attempted once.
- Updated current runtime docs while stating explicitly that native clients and
  product configuration are not yet wired to the new foundation.

Verification:

- `cargo test -p rove-models` - passed, 86 tests, 0 failures.
- `cargo fmt --all --check` - passed.
- `cargo clippy -p rove-models --all-targets -- -D warnings` - passed.
- `cargo test --workspace` - passed across all Rust Workspace packages,
  integration tests, and doc tests.
- Focused Transport suite - 7 tests passed, including local mock HTTP auth,
  fragmented SSE, bounded/redacted error bodies, idle timeout, endpoint/header
  rejection, config bounds, and Unicode-safe truncation.

Files introduced or changed in this stage:

- `models/src/error.rs`
- `models/src/routing.rs`
- `models/src/provider/auth.rs`
- `models/src/provider/transport.rs`
- `models/src/provider/framing.rs`
- `models/src/provider/mod.rs`
- `docs/runtime/subsystems.md`
- `docs/runtime/implementation-guide.md`
- `docs/runtime/implementation-status.md`
- the design and progress documents

Next:

- Implement `ProviderClient` and the OpenAI Chat strategy/decoder. Keep the old
  client until deterministic parity tests pass; do not change bootstrap yet.

### 2026-07-23 - Stage 3 complete

Completed:

- Added `ProviderClient`, which composes resolved target configuration, a
  `WireProtocol`, and the shared `Transport` behind the existing `ModelClient`
  contract while preserving endpoint-aware legacy target identity.
- Added the native OpenAI Chat request strategy and per-request stream decoder
  for messages, tool schemas/history, provider options, fragmented tool calls,
  usage, done events, and typed HTTP errors.
- Added a fragmented local HTTP/SSE test covering the complete shared path,
  request URL, bearer injection, native tool-call reconstruction, usage, done,
  and target identity.
- Added direct migration parity tests that execute both the legacy and new
  request builders, event normalizers, and HTTP error classifiers on the same
  fixtures. The old client remains compiled and production assembly is still
  unchanged.

Verification:

- `cargo test -p rove-models` - passed, 95 tests, 0 failures.
- `cargo clippy -p rove-models --all-targets -- -D warnings` - passed.
- `cargo fmt --all --check` - passed.
- `cargo test --workspace` - passed across all Rust Workspace packages,
  integration tests, and doc tests.
- `git diff --check` - passed.

Files introduced or changed in this stage:

- `models/src/openai.rs`
- `models/src/provider/client.rs`
- `models/src/provider/protocols/mod.rs`
- `models/src/provider/protocols/openai_chat.rs`
- `models/src/provider/transport.rs`
- `models/src/provider/mod.rs`
- the design and progress documents

Next:

- Migrate OpenAI Responses into a dedicated strategy/decoder and prove request,
  cache-option, stream, tool, usage, terminal-error, HTTP-error, and identity
  parity before changing bootstrap assembly.

### 2026-07-23 - Stage 4 complete

Completed:

- Added a native OpenAI Responses strategy with the `/responses` request
  shape, instructions/input items, function tools and history, provider
  options, and optional prompt-cache key/retention fields.
- Added a per-request Responses decoder for text deltas, fragmented function
  arguments, deduplicated function completion, cached-token usage, done, and
  terminal failed/incomplete events.
- Kept streamed terminal failures typed while preventing provider-controlled
  failure text from being echoed through the new client error.
- Added direct request, decoder-event, HTTP-error, and target-identity parity
  tests against the legacy Responses client plus a fragmented local HTTP/SSE
  test through `ProviderClient` and the shared Transport.
- Updated current runtime documentation to distinguish implemented migration
  code from the still-legacy product configuration and assembly path.

Verification:

- `cargo test -p rove-models` - passed, 101 tests, 0 failures.
- `cargo clippy -p rove-models --all-targets -- -D warnings` - passed.
- `cargo fmt --all --check` - passed.
- `cargo test --workspace` - passed across all Rust Workspace packages,
  integration tests, and doc tests.
- `git diff --check` - passed.

Files introduced or changed in this stage:

- `models/src/openai_responses.rs`
- `models/src/provider/protocols/mod.rs`
- `models/src/provider/protocols/openai_responses.rs`
- `docs/runtime/subsystems.md`
- `docs/runtime/implementation-guide.md`
- `docs/runtime/implementation-status.md`
- the design and progress documents

Next:

- Migrate Anthropic Messages and Ollama Chat into distinct native protocol
  strategies/decoders with direct parity and shared-transport tests. Do not
  switch product assembly until both are green.

### 2026-07-23 - Stage 5 complete

Completed:

- Added a native Anthropic Messages strategy with `/v1/messages`, `x-api-key`,
  `anthropic-version`, system extraction, tool-use/tool-result blocks, provider
  options, incremental tool JSON, cumulative usage, and terminal events.
- Added a native Ollama Chat strategy with `/api/chat`, no-auth default,
  Ollama roles/tool history/options, JSONL text/tool events, usage, and done.
- Added typed, non-echoing failures for malformed or explicit provider stream
  errors while retaining direct HTTP error classification parity.
- Added direct request, event, HTTP-error, and identity parity tests against
  both legacy clients plus fragmented local HTTP tests through the shared
  Transport. Ollama coverage includes a final JSONL record without a newline.
- Updated current runtime documentation to list all four implemented native
  protocol strategies while retaining the explicit not-yet-assembled status.

Verification:

- `cargo test -p rove-models` - passed, 111 tests, 0 failures.
- Anthropic focused protocol suite - passed, 5 tests, 0 failures.
- Ollama focused protocol suite - passed, 5 tests, 0 failures.
- `cargo clippy -p rove-models --all-targets -- -D warnings` - passed.
- `cargo fmt --all --check` - passed.
- `cargo test --workspace` - passed across all Rust Workspace packages,
  integration tests, and doc tests.
- `git diff --check` - passed.

Files introduced or changed in this stage:

- `models/src/anthropic.rs`
- `models/src/ollama.rs`
- `models/src/provider/protocols/anthropic.rs`
- `models/src/provider/protocols/ollama.rs`
- `models/src/provider/protocols/mod.rs`
- `docs/runtime/subsystems.md`
- `docs/runtime/implementation-guide.md`
- `docs/runtime/implementation-status.md`
- the design and progress documents

Next:

- Introduce validated named profiles and bounded env/file secret resolution,
  add deterministic legacy conversion, register built-in protocols, and use
  `ProviderClient` for primary/fallback bootstrap assembly without changing
  API inventory or Web yet.

### 2026-07-23 - Stage 6 checkpoint: profile foundation

Completed so far:

- Added named profile configuration with explicit active/fallback references,
  open wire protocol IDs, endpoint/model/options data, custom headers, and
  protocol-specific options.
- Added redacted environment/file secret references with 16 KiB secret, 64
  KiB protocol-option, and 64-header bounds. Relative secret files are bounded
  to the workspace unless external paths are explicitly enabled.
- Added the default registry containing OpenAI Chat, OpenAI Responses,
  Anthropic Messages, and Ollama Chat.
- Added profile-aware environment layers and model override precedence while
  rejecting ambiguous named/legacy fallback combinations.
- Added Transport authentication-header validation that permits credential
  headers but rejects host, connection, content length, transfer encoding, and
  content type overrides.

Verification so far:

- `cargo check -p rove-app-bootstrap` - passed after the profile foundation.
- Focused Transport authentication-header test - passed, 1 test, 0 failures.

Next at checkpoint:

- Replace the legacy-client factory path with registry-resolved
  `ProviderClient` targets, retain Fake locally, add fallible and compatible
  construction APIs, then complete the Stage 6 config/redaction/identity test
  matrix.

### 2026-07-24 - Stage 6 complete

Completed:

- Replaced bootstrap assembly so native HTTP primary/fallback targets resolve
  through the protocol registry into `ProviderClient`. Fake remains local.
- Kept the previous infallible `build_model_client*` surface for compatibility
  and added `try_build_*` constructors that fail at assembly with typed
  configuration errors.
- Added named-profile identity, unknown-protocol fail-closed, custom registry
  injection, secret file/path bounds, Debug/`dump-config` redaction, and
  config precedence coverage.
- Updated current runtime docs so bootstrap assembly is described as already
  using `ProviderClient`, while API inventory and Web still use the legacy
  profile shape.
- Fixed legacy OpenAI Responses conversion so unset
  `responses_prompt_cache_retention` is omitted instead of serialized as JSON
  null. The wire option parser also treats explicit null as unset so optional
  string fields cannot fail closed on absent values.

Verification:

- `cargo fmt --all --check` - passed (after `cargo fmt --all`).
- `cargo clippy -p rove-models --all-targets -- -D warnings` - passed.
- `cargo clippy -p rove-app-bootstrap --all-targets -- -D warnings` - passed.
- `cargo test -p rove-models` - passed, 112 tests, 0 failures.
- `cargo test -p rove-app-bootstrap` - passed, 21 tests, 0 failures.
- `cargo test -p rove-integration-tests --test model_factory` - passed, 11
  tests, 0 failures.
- `cargo test -p rove-integration-tests --test cli_config` - passed, 2 tests,
  0 failures.
- `cargo test --workspace` - passed after the Responses null-option fix,
  including `api_jobs_accept_openai_responses_provider_profile_per_request`.
- `git diff --check` - passed.

Files introduced or changed for Stage 6 completion (plus earlier Stage 6
checkpoint work):

- `apps/bootstrap/src/factory.rs`
- `apps/bootstrap/src/provider.rs`
- `apps/bootstrap/src/config.rs`
- `apps/bootstrap/src/lib.rs`
- `apps/bootstrap/Cargo.toml`
- `apps/cli/src/cli/config.rs`
- `models/src/provider/**`
- `models/src/{lib,error,routing,openai,openai_responses,anthropic,ollama}.rs`
- `models/src/provider/protocols/openai_responses.rs`
- `tests/model_factory.rs`
- `tests/cli_config.rs`
- `docs/runtime/{implementation-guide,implementation-status,subsystems}.md`
- the design and progress documents

Next:

- Stage 7: converge API `ProviderProfileRequest` / inventory and Web provider
  controls onto open wire-protocol identity while preserving the legacy
  provider-name aliases and secret-env-only contract.

### 2026-07-24 - Stage 7 complete

Completed:

- Extended API `ProviderProfileRequest` with optional `wire_protocol` while
  keeping legacy `name` aliases (`openai-compatible`, `anthropic`, …).
- Normalized request profiles into request-scoped named profiles that assemble
  through bootstrap `ProviderClient` rather than rewriting only flat legacy
  fields.
- Added request-scoped `SecretSource::Literal` so API job construction can
  resolve env secrets once at request time (tests may clear env vars before the
  async job starts) without persisting secret values in durable config.
- Inventory routing now keys off open protocol family identity while still
  accepting legacy display names.
- Web types and workbench send both `name` and `wire_protocol`; provider labels
  reflect the open protocol family.
- `/providers/test` responses include the resolved wire protocol.
- Legacy factory name aliases accept open protocol ids such as `openai-chat`
  and `anthropic-messages`.

Verification:

- `cargo fmt --all` - passed.
- `cargo clippy -p rove-models -p rove-app-bootstrap -p rove-api -p rove-cli --all-targets -- -D warnings` - passed.
- `cargo test -p rove-integration-tests --test api` - passed, 52 tests.
- `cargo test -p rove-integration-tests --test model_factory --test cli_config` - passed.
- `cargo test -p rove-app-bootstrap` - passed.
- `cargo test --workspace` - passed.
- Web unit tests were not run: local `apps/web/node_modules` / `vitest` are not
  installed in this worktree. Type-level contract updates are covered by the
  source changes and TypeScript fixtures.

Files introduced or changed:

- `apps/api/src/provider.rs`
- `apps/api/src/types.rs`
- `apps/api/src/lib.rs`
- `apps/bootstrap/src/provider.rs`
- `apps/bootstrap/src/factory.rs`
- `apps/cli/src/cli/config.rs`
- `apps/web/lib/rove-types.ts`
- `apps/web/lib/rove-types.test.ts`
- `apps/web/components/rove-workbench.tsx`
- design and progress documents

Next:

- Stage 8: opt-in bounded external adapter v1 for unsupported wire formats.

### 2026-07-24 - Stage 8 complete

Completed:

- Added `ExternalAdapterClient` / `ExternalAdapterConfig` under
  `models/src/provider/external_adapter.rs` for protocol id
  `external-adapter-v1`.
- Process boundary uses a direct executable argv (never a shell string),
  env allowlist + explicit env map, optional working directory, bounded
  startup/request/idle/shutdown timeouts, bounded stdout/stderr, and
  kill-on-drop child cleanup.
- Stream protocol is versioned JSONL: `hello`/`hello_ok` then `stream`, then
  normalized events (`text_delta`, tool events, `usage`, `done`, `error`).
- Secrets are injected into the request payload without logging; adapter error
  text is not echoed into `ModelError` Display beyond stable classes.
- Bootstrap profile validation and factory assembly route
  `external-adapter-v1` into the process client instead of HTTP Transport.
- Deterministic fixture binary under
  `models/tests/fixtures/external_adapter_v1_fixture.rs` covers happy path,
  malformed stream, idle timeout, secret delivery, and path/shell negatives.

Verification:

- `cargo clippy -p rove-models -p rove-app-bootstrap --all-targets -- -D warnings` - passed.
- `cargo test -p rove-models provider::external_adapter` - passed, 5 tests.
- `cargo test -p rove-models` - passed, 117 tests.
- `cargo test -p rove-app-bootstrap` - passed, 21 tests.
- `cargo test -p rove-integration-tests --test model_factory` - passed, 11 tests.
- `cargo test --workspace` - passed.

Files introduced or changed:

- `models/src/provider/external_adapter.rs`
- `models/src/provider/mod.rs`
- `models/tests/fixtures/external_adapter_v1_fixture.rs`
- `apps/bootstrap/src/factory.rs`
- `apps/bootstrap/src/provider.rs`
- design and progress documents

Next:

- Stage 9: cleanup/release evidence — decide legacy client retirement scope,
  refresh current runtime docs for the adapter boundary, and collect workspace
  / integration / optional real-provider gate evidence.

### 2026-07-24 - Stage 9 complete

Completed:

- Documented the compatibility decision: legacy modules
  `models/src/openai.rs`, `openai_responses.rs`, `anthropic.rs`, and
  `ollama.rs` stay in-tree only as parity-test and transition references.
  Production `ModelClientFactory` assembly does not construct those HTTP
  clients; it uses `ProviderClient` and the opt-in external adapter.
- Refreshed current-runtime docs so they describe named profiles, API/Web open
  wire-protocol contracts, bootstrap assembly, and `external-adapter-v1`.
- Collected deterministic release-gate evidence for the Provider redesign
  workspace.

Verification:

- `cargo fmt --all --check` - passed (after `cargo fmt --all`).
- `cargo clippy --all-targets -- -D warnings` - passed.
- `cargo test --workspace` - passed.
- `git diff --check` - passed for the Provider worktree changes.
- Web gates (`pnpm test` / typecheck / build) - not run; this worktree has no
  `apps/web/node_modules` install. Type-level Web contract updates from Stage 7
  remain in source; install-and-run is a follow-up on a machine with Node deps.
- Opt-in real-provider smoke - skipped; no `OPENAI_API_KEY` /
  `ANTHROPIC_API_KEY` present in the environment for this run.
- Deterministic `scripts/integration-smoke.ps1` full local integration - not
  re-run in this turn; the workspace unit/integration Rust suite already covers
  API provider profile jobs, factory assembly, and adapter fixtures. Full
  scripted smoke remains recommended before merge if CI does not already run it.

Files changed for Stage 9:

- `docs/runtime/subsystems.md`
- `docs/runtime/implementation-guide.md`
- `docs/runtime/implementation-status.md`
- `docs/runtime/release-readiness.md`
- `docs/design/2026-07-23-provider-layer-redesign-design.md`
- `docs/plans/2026-07-23-provider-layer-redesign.md`

Next:

- Human review of the uncommitted `feature/provider-layer-redesign` worktree.
- Commit/PR when ready. Optional follow-ups: install Web deps and run pnpm
  gates; run `scripts/integration-smoke.ps1`; delete legacy native clients after
  an explicit compatibility-window decision.

### 2026-07-24 - Channel UX follow-up

Completed after Stage 9:

- User-facing API/Web profiles prefer a **type** preset
  (`openai`, `openai-responses`, `anthropic`, `ollama`, `fake`) that maps to
  an internal `wire_protocol`. Official and relay endpoints share the same type.
- Optional display `name` defaults from `api_base` host when omitted.
- Legacy clients that put protocol aliases (`openai-compatible`, …) in `name`
  or `channel` are **not** supported; use the five canonical types only.
- Web workbench: Type selector (label **OpenAI**, not “compatible”) + optional
  label + API base + key env + model.
- `/providers/test` returns `channel` and `wire_protocol` alongside the
  host-derived display label.

Verification:

- `cargo test -p rove-api --lib` - passed (includes channel normalization tests).
- `cargo test -p rove-integration-tests --test api` - passed for provider tests.
- `cargo clippy -p rove-api --all-targets -- -D warnings` - passed.

Files changed:

- `apps/api/src/provider.rs`, `types.rs`, `lib.rs`
- `apps/web/lib/rove-types.ts`, `rove-types.test.ts`, `rove-client.test.ts`
- `apps/web/components/rove-workbench.tsx`
- `apps/web/tests/e2e/workbench.spec.ts`
- `tests/api.rs`
- runtime docs and this ledger

### 2026-07-24 - Remove user-facing legacy type aliases

Per product decision, the API/Web request surface accepts **only** the five
canonical types; the old compatibility aliases were dropped (they were never
requested by real clients and only added surface area):

- `provider.channel` accepts only `openai`, `openai-responses`, `anthropic`,
  `ollama`, `fake`. Removed aliases: `openai-compatible`, `openai-chat`,
  `responses`, `anthropic-messages`, `ollama-chat`, `gemini`,
  `gemini-openai-compat`.
- The "type alias in `name`" fallback is gone. `name` is now purely a display
  label; the type must come from `channel` (or the advanced `wire_protocol`).
- Gemini relays with OpenAI-compatible APIs are reached via the `openai` type;
  there is no separate Gemini type.
- Web `ProviderChannel` union trimmed to the five types; dead `case` arms and
  the "OpenAI-compatible" default label removed from the workbench.
- Internal `wire_protocol` ids (`openai-chat`, `anthropic-messages`,
  `ollama-chat`, …) are unchanged — they are the stable protocol identity, not
  user-facing aliases.

### 2026-07-24 - Drop bootstrap/config/script `openai-compatible` name

Also removed the old product name from flat config, routing labels, smoke
scripts, and active runtime docs:

- `canonical_provider_name` / flat `[provider].name` / `fallback_providers`
  accept only `openai`, `openai-responses`, `anthropic`, `ollama`, `fake`
  (no `openai-compatible`, no `responses` alias).
- `protocol_client_namespace("openai-chat")` now returns `openai`.
- Legacy parity `OpenAiClient::client_id` namespace is `openai`.
- `scripts/provider-integration.ps1`, `.env.integration.example`, README, and
  current `docs/runtime/*` examples use `openai`.

Verification:

- `cargo fmt --all`
- `cargo test -p rove-api --lib` — 11 passed
- `cargo test -p rove-app-bootstrap --lib` — 21 passed
- `cargo test -p rove-integration-tests --test model_factory --test code_hygiene` — 11 + 22 passed
- `cargo test -p rove-models --lib provider::client` — 2 passed
- `cargo test -p rove-cli --lib` — 151 passed
- `cargo test -p rove-runtime --lib runtime_identity` — 5 passed
- `cargo test -p rove-integration-tests --test api` — 52 passed
- `cargo test -p rove-integration-tests --test cli_config` — 2 passed
- `cargo clippy -p rove-api -p rove-app-bootstrap -p rove-models --all-targets -- -D warnings` — passed

Files changed:

- `apps/bootstrap/src/{config,factory,provider}.rs`
- `models/src/openai.rs`, `models/src/provider/client.rs`,
  `models/src/provider/protocols/openai_chat.rs`
- `runtime/src/foundation/runtime_identity.rs`
- `tests/{model_factory,provider_smoke,code_hygiene}.rs`
- `apps/cli/src/cli/ui.rs`
- `scripts/provider-integration.ps1`, `.env.integration.example`, `README.md`
- active `docs/runtime/*` and this ledger

### 2026-07-24 - Provider model catalog endpoint

Added a dedicated model-list API separate from connectivity/model-presence
testing:

- `POST /providers/models` returns `models: string[]` for a typed provider
  profile (`channel`/`wire_protocol` + `api_base` + optional `api_key_env`).
- `POST /providers/test` remains the selected-model readiness check
  (`model_present`, key presence, inventory count).
- Web workbench: **Load models** fills a model datalist; **Test** still checks
  the currently selected model.
- Secrets stay server-side via `api_key_env`; raw keys are never returned.

Verification:

- `cargo test -p rove-api --lib` — 11 passed
- `cargo test -p rove-integration-tests --test api -- api_lists_provider_models ...` — passed
- `cargo clippy -p rove-api --all-targets -- -D warnings` — passed

Files:

- `apps/api/src/{types,lib,docs}.rs`
- `tests/api.rs`
- `apps/web/lib/{rove-types,rove-client,rove-client.test}.ts`
- `apps/web/components/rove-workbench.tsx`
- `apps/web/tests/e2e/workbench.spec.ts`
- `docs/runtime/subsystems.md`

