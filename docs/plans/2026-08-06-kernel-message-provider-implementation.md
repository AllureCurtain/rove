# Kernel, Message, and Provider Implementation

> Status: **First parallel wave implemented on this branch; later stages remain proposed**
>
> Prerequisite implementation: `d2cd822` (`feat(security): gate workspace
> project activation`)
>
> Worktree: `.worktrees/kernel-message-provider`
>
> Initial branch: `feature/kernel-message-provider-wave1`
>
> Start gate: the branch must be created from the current `origin/main`, must
> contain prerequisite `d2cd822` and this brief, and must have a clean worktree.
>
> This document describes the target work and the stop boundary for this branch.
> The prerequisite implementation named above is present, and the first parallel
> wave below is implemented here. The implementation notes distinguish the
> runtime-connected behavior from the foundation-only and later work that must
> wait for a refreshed `main` baseline.

### First-wave implementation status

Implemented in this branch:

- schema-1 provider-neutral `Session` entries with explicit internal-call and
  provider-wire identity, validation, deterministic legacy migration, and
  bounded atomic suffix projection;
- canonical session persistence in `PromptCheckpoint`/`TaskState` snapshots,
  artifact dual-read for old `Vec<Message>` histories, and bounded resume
  projection through the selected provider protocol. The full canonical
  session remains durable truth, while resumed prompts receive only the
  correlation-safe canonical suffix with a 12-entry target (expanded only to
  keep a tool round atomic) plus the checkpoint summary;
- normalized assistant turns, usage, stop reasons, stream assembly, capability
  checks, and provider-specific wire-ID projection for OpenAI Chat, OpenAI
  Responses, Anthropic, Ollama, and Fake;
- runtime and integration coverage for restart/resume, provider switching,
  native multi-tool compaction/trim, canonical sessions longer than the
  checkpoint tail, legacy artifacts, stop-reason mapping, and malformed streams
  that must execute zero tools.

The runtime still uses the typed session's bounded `Vec<Message>` projection at
the existing context-manager boundary. The follow-up authoritative Tool Schema
and Runtime capability snapshot foundation is implemented by
[`2026-08-07-authoritative-tool-schema-runtime-validation.md`](2026-08-07-authoritative-tool-schema-runtime-validation.md).
The one shared Agent-kernel cutover, lifecycle finalization/evaluator work, and
AgentDefinition/procedural-knowledge work remain **Proposed / Not Implemented**.

## 1. Objective

Evolve rove toward one provider-neutral Agent kernel without discarding its
durable event, state, approval, recovery, and product contracts. This work owns
typed message/session projection, provider protocol normalization, the shared
kernel migration, and later strategy/Agent-definition integration.

For the first parallel wave, complete only:

1. typed message and session projection;
2. provider-neutral turn/result/stop contracts;
3. request history and tool-call identity projection;
4. shared streaming assembly for the currently supported native providers.

Stop after the first-wave exit gate. Tool-schema compilation, the one-kernel
cutover, lifecycle finalization, and Agent-definition work require their named
dependencies to be merged into `main` and a fresh branch baseline.

## 2. Required orientation

Read before editing:

1. [`../../AGENTS.md`](../../AGENTS.md)
2. [`../ONBOARDING.md`](../ONBOARDING.md)
3. [`../runtime/README.md`](../runtime/README.md)
4. [`../runtime/react-loop.md`](../runtime/react-loop.md)
5. [`../runtime/provider-smoke.md`](../runtime/provider-smoke.md)
6. [`../design/2026-07-14-agent-execution-lifecycle-design.md`](../design/2026-07-14-agent-execution-lifecycle-design.md)
7. [`../design/2026-07-14-agent-definition-and-procedural-knowledge-design.md`](../design/2026-07-14-agent-definition-and-procedural-knowledge-design.md)

Code and tests remain authoritative when a design example is stale.

## 3. Sealed contracts

### 3.1 Ownership

- `rove-core` owns provider-neutral in-memory model/tool iteration and the
  smallest embedding contract.
- `rove-models` owns provider clients, wire protocols, request projection,
  streaming assembly, provider capability checks, and wire IDs.
- `rove-runtime` owns durable policy, event translation, context, planning,
  state, recovery, resume, and the product Engine facade.
- CLI, API, Web, and benchmark paths must consume the shared Runtime/Kernel;
  they must not create another Agent loop.

### 3.2 Canonical messages and session entries

Canonical state is provider-neutral and is never a persisted provider request
body. The target model supports, at minimum:

- ordered text and bounded rich-content references;
- assistant turns with zero or more typed tool calls;
- tool results correlated to canonical call IDs;
- application/session entries for user input, assistant output, tool activity,
  controls, compaction, and future capability hydration;
- explicit internal ID versus provider wire ID projection.

Compatibility rules:

- existing `Message` and `TaskState` artifacts remain readable;
- additive fields have defaults;
- old tool results without native IDs are accepted only through a deterministic
  compatibility projection;
- duplicate, empty, or conflicting IDs fail before tool execution;
- session history never stores OpenAI-, Anthropic-, or Ollama-specific payloads.

### 3.3 Provider protocol

The normalized provider result is an assistant turn, not a bag of strings.
Provider protocols must agree on:

- ordered content and tool calls;
- usage accounting;
- normalized stop reasons;
- incomplete/malformed stream failure;
- canonical-to-wire ID mapping;
- capability negotiation before network use;
- deterministic tool-schema/request signatures.

Wire-specific cache markers, reasoning/signature blocks, headers, and endpoint
fields stay inside `rove-models`. A provider switch may rewrite wire IDs but
must not mutate canonical history or break call/result correlation.

### 3.4 Lifecycle and extension authority

The later kernel cutover preserves one ordered lifecycle:

```text
context build
  -> before-model extensions
  -> model stream assembly
  -> assistant-turn validation
  -> tool policy/approval/execution
  -> result writeback
  -> durable event/state projection
  -> evaluator/finalizer
```

Extension output is data under the invoking authority. It cannot grant tool
permission, project trust, provider credentials, or a larger budget. Hook
ordering is deterministic, failures are typed, and no compatibility mode may
double-call a model or replay a mutation.

### 3.5 Durable truth and derivation

| Concern | Canonical owner | Derived consumers |
|---|---|---|
| Runtime lifecycle facts | canonical `StreamEvent` trace | SQLite event index, SSE, Web projection, report |
| Resumable execution state | `TaskState` plus prompt checkpoint | resume, repair, session continuation |
| Provider-neutral conversation | typed session/application entries | context projection, provider request history, transcript |
| Step/plan history | append-only records, decisions, revisions | task state, report, UI |
| Provider wire identity | model-layer projection metadata | target request/replay only |

`report.json` remains derived. Provider requests and UI read models never
become independent durable truth.

### 3.6 Schema and migration workflow

- Rust types are the source for runtime and API contracts.
- Utoipa OpenAPI generation and contract tests verify public Rust surfaces.
- Strict Web parsers/types change in the same commit as a public API field.
- Do not add a second schema generator during this wave.
- Readers become dual-compatible before writers emit a new schema version.
- New writers are single-path and feature-gated where rollback needs it.
- Rollback must never require replaying completed model calls or mutations.

### 3.7 Quantitative non-regression floor

- all deterministic Rust and Web tests pass;
- all default no-network benchmark tasks pass;
- provider request/stream fixtures remain byte-bounded and exact;
- malformed, duplicate, truncated, or oversized tool calls execute zero tools;
- context hard/soft/reserved limits do not increase silently;
- no new external provider interoperability claim is made without its opt-in
  gate.

## 4. File ownership

Owned in this worktree for the first wave:

- `models/`
- `core/`
- `runtime/src/foundation/session.rs`
- `runtime/src/foundation/mod.rs`
- provider-neutral additions under `runtime/src/foundation/`
- additive session compatibility work under `runtime/src/state/`
- focused package tests beside those modules
- `tests/embedding_contract.rs`
- `tests/model_factory.rs`
- `tests/provider_smoke.rs`
- `tests/artifact_compatibility.rs`
- new narrowly named kernel/provider integration tests
- `docs/runtime/react-loop.md`
- `docs/runtime/provider-smoke.md`

Do not modify during the first wave:

- `apps/bootstrap/`
- `apps/api/` or `apps/web/`
- `runtime/src/tools/`
- `runtime/src/lib.rs`
- Project Trust or Execution Environment modules
- ProductStore tables/migrations
- root `Cargo.toml` or `Cargo.lock`
- `PRODUCT_ACCEPTANCE_REPORT.json`
- current runtime documents other than the two explicitly owned above

Canonical event families, broad `TaskState` restructuring, generated public
schemas, and root dependency changes require an explicit shared-hotspot
assignment before editing. Additive fields needed solely for typed-session
backward compatibility are allowed, but must retain old fixture reads.

## 5. First parallel wave

### Checkpoint 1 - Characterization and exact types

- Freeze existing provider request/stream behavior with focused fixtures.
- Define exact provider-neutral assistant turn, content, call, result, stop, and
  identity types.
- Define typed session/application entries and validation rules.
- Record schema version, defaults, maximum sizes, ordering, and unknown-field
  behavior in code and tests.

Exit:

- every serialized type has a compatibility test;
- invalid identity/correlation fixtures fail before execution;
- no provider wire type escapes `rove-models`.

### Checkpoint 2 - Typed message/session projection

- Add dual-read compatibility from current `Message`/history artifacts.
- Project canonical entries into context messages without orphan tool results.
- Persist enough canonical identity for exact resume and provider switching.
- Keep compaction and transcript projections deterministic and bounded.

Exit:

- old artifact fixtures still load;
- current and migrated histories produce equivalent safe context;
- native multi-tool rounds remain atomic under trim/compaction;
- restart/resume does not invent or duplicate a tool result.

### Checkpoint 3 - Provider protocol foundation

Implement these independently reviewable slices:

1. normalized assistant turn, stop reason, and usage output;
2. request/history projection with canonical/wire ID mapping;
3. shared stream assembly and tool-call correlation for OpenAI Chat
   Completions, OpenAI Responses, Anthropic, Ollama, and Fake.

Exit:

- existing native-provider body and stream fixtures pass;
- switching providers preserves canonical history;
- malformed/incomplete streams cannot reach ToolRegistry;
- capability failure happens before network use;
- no external-provider claim is added unless the corresponding gate ran.

## 6. Required verification

Run focused checks first, then the full gate before handoff:

```powershell
cargo fmt --all --check
cargo test -p rove-models
cargo test -p rove-core
cargo test -p rove-integration-tests --test embedding_contract
cargo test -p rove-integration-tests --test model_factory
cargo test -p rove-integration-tests --test provider_smoke
cargo test -p rove-integration-tests --test artifact_compatibility
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Run provider opt-in gates only when credentials and explicit authorization are
present. A skip is not evidence of interoperability.

## 7. Handoff and stop condition

Produce separate commits for the three checkpoints. At handoff report:

- commit SHAs and exact base SHA;
- files changed;
- serialized compatibility story;
- current runtime documentation updated for behavior changed in this wave;
- tests and real exit codes;
- optional gates not run;
- unresolved risks and any requested shared-hotspot change;
- clean `git status --short`.

This first-wave stop condition required a refreshed merged baseline before any
tool-schema work. That prerequisite was satisfied at `559bc1e`; the bounded
Tool Schema/capability snapshot follow-up is tracked separately. The
Runtime-to-Core loop cutover, Finalizer/evaluator work, and AgentDefinition/Skill
implementation remain stopped.

## 8. Later owned work after refresh

Later checkpoints remain assigned here but are not part of the first handoff:

- rich content/replay metadata beyond the implemented authoritative bounded
  Tool Schema validation, registration pinning, pre-dispatch negotiation, and
  Runtime capability snapshot foundation;
- one shared Agent kernel and extension plane;
- rule-first ambiguity evaluation, independent Finalizer, global budgets, and
  trace-tail reconciliation;
- strategy/context-efficiency migration;
- AgentDefinition, immutable runtime profiles, instructions, and typed
  procedure/Skill identity.
