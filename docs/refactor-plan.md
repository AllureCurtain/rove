# Structural refactor plan

Updated: 2026-10-01

Records the structural refactors to do: why, what, in what order, and how to verify. Progress is tracked in TODO.md; settled technical decisions go into docs/decisions.md.

## Background (measured 2026-10-01)

| Metric | Value |
| --- | --- |
| Rust code | ~231k lines, 313 files; about 40% is tests (`tests/` 40k lines, in-crate `#[cfg(test)]` ~51k lines) |
| Files over 800 lines | 78; largest are `tests/api.rs` 20k lines, `apps/api/src/product/store/repository.rs` 8450 lines, `apps/api/src/lib.rs` 7420 lines (127 functions) |
| Dependency graph | 716 packages; `rove-desktop` depends on 394, `rove-api` on 248 |
| target directory | 13 GB: 6.4 GB incremental cache, 2.2 GB PDB, 2.6 GB rlib/rmeta |
| `cargo check -p rove-api -p rove-cli` after one runtime line changed | ~65s |
| `cargo test --workspace --no-run` after one runtime line changed | ~91s, recompiling 73 rove units, ~656s cumulative CPU |
| `cargo check -p rove-api` after an api-only change | ~59s |
| Change counts since 2026-08 (by file) | `apps/api` 360, `tests` 167, `apps/cli` 127, runtime submodules 13–98 |

Conclusions:
- The slowdown comes from the shape of the code, not its size. Two giant crates (`rove-runtime` 70k lines, `rove-api` 56k lines) plus 27 integration test binaries that all link the full stack mean one change forces recompilation and relinking of a large area.
- The most-changed area is `apps/api`; starting there gives the most direct payoff.

Already tried, no effect: switching the linker to `rust-lld` in `.cargo/config.toml`. The test build after a runtime change went from 91s to 102s (within noise), disk usage unchanged; reverted. Linking is not the bottleneck.

## Principles

- Each step is an independent PR: separately verifiable, separately revertible, and behavior-preserving (API, events, and storage formats unchanged).
- Each step measures incremental compile time before and after (fixed commands, see "Verification"), recorded in the PR description. Steps with no clear benefit stop — do not force them through.
- Extracted crates must also respect one-way dependencies, and `crates/tests/workspace_architecture.rs` allowlists get updated.
- Moving code means moving only; renames and logic refactors go in separate PRs.

## Phase 1: do now

### 1. Merge integration test binaries from 27 to a few (done 2026-10-01)

- Result: 27 merged into 3 (`it`, `api`, `secret_authority_unarmed`); all 483 tests still run. `cargo test --workspace --no-run` after a runtime change dropped from 146s to 65s; rebuild after an integration-test-only change dropped from 148s to 40s.
- Found while merging: CLI process tests had `target/debug/rove` hardcoded and failed when `CARGO_TARGET_DIR` was set; fixed by locating the binary relative to the test executable in shared `tests/support`.

- Before: `tests/Cargo.toml` defined 27 `[[test]]` targets, each a standalone executable linking the full api/cli/bench/runtime stack.
- Approach:
  - Merge into `tests/it/main.rs` plus submodules; groups needing their own process environment stay separate (e.g. `api.rs` mutates env vars, `stress` is opt-in). Target: no more than 3 binaries.
  - Targeted runs switch to filters: `cargo test -p rove-integration-tests --test it api::`.
  - Update `--test <name>` usage in docs/development.md, CI, and scripts (`scripts/provider-integration.ps1`, etc.).
- Risk: tests that mutate env vars or process-global state may interfere once merged; must audit first (`tests/api.rs` contains `set_var`).
- Done criteria: `cargo test --workspace` all green; test binary count down; measured `--no-run` time after a runtime change is lower.

### 2. Extract ProductStore into `rove-product-store` (done 2026-10-01)

- Result: new crate `apps/product-store`, ~25k lines (store, contracts, cursor, ownership, pricing, secret_patterns, plus pure path and truncation helpers pulled from api). api keeps original paths via `pub use rove_product_store::*`. After an api-routes-only change: `rove_api` lib test unit 26s → 10s, lib unit 16.6s → 5.8s, `cargo check -p rove-api` ~9s → ~5s; store-only changes build store tests in ~10s.
- Path rules for attachments and workspace files (`join_safe`, etc.) were extracted into pure functions returning `String` errors, wrapped in `ApiError` on the api side — one implementation shared by both.

- Before: `apps/api/src/product/store/` ~20k lines (37 tables, schema v22), depending on api only via `crate::product` (types and `ownership`), `crate::product::export`, `crate::product::attachments`, and one function in `crate::pricing`.
- Approach:
  - Create `apps/product-store` depending on `rove-runtime` and `rusqlite`.
  - Push down the product domain types the store uses. If too many, first split out `rove-product-types`.
  - api re-exports to keep original paths; call sites unchanged.
- Payoff: the largest single file `repository.rs` and 4300 lines of store tests leave api's compile unit; api route changes no longer recompile the store, and vice versa.
- Done criteria: api's crate dependency allowlist updated; store tests pass in the new crate; `cargo check -p rove-api` after an api-only change measurably faster.

### 3. Move HTTP-unrelated dependencies out of rove-api (measured no benefit, abandoned 2026-10-01)

- Result: built the `native-picker` and `bench` features; under `--no-default-features` the dependency tree does drop rfd and rove-bench, but only 6 crates disappear (231 → 225) because api already had rove-bench's dependencies. After a runtime change, `cargo check -p rove-api` measured 13–14s with defaults and 14–15s without — no difference. Per "stop when the benefit is unclear", not merged; avoids maintaining an extra feature matrix.

- `rfd` (native folder dialog) is only used by `product/workspace_picker.rs`.
  - Abstract it behind an injected `FolderPicker` trait: Desktop supplies the rfd implementation; standalone api defaults to rfd behind a default-on `native-picker` cargo feature.
  - Existing `ROVE_DISABLE_NATIVE_FOLDER_PICKER` behavior unchanged.
- `rove-bench` is only used by `benchmark.rs` (6 `/bench/*` routes); put it behind a default-on `bench` feature. CI and day-to-day dev could turn it off.
- Done criteria: `cargo tree -p rove-api --no-default-features` contains neither rfd nor rove-bench; default build behavior and OpenAPI unchanged.

### 4. Split `apps/api/src/lib.rs` by responsibility (done 2026-10-01)

- Result: 7419 lines split into `lib.rs` (613 lines: type declarations, `router`, `serve*`) plus ten modules — `state`, `jobs`, `review`, `followup`, `launch`, `fork`, `supervisor`, `assembly`, `events`, `error` — with tests moved to `tests.rs`. Move only; OpenAPI is byte-identical before and after. `supervisor.rs` is still 1362 lines and can be split further later.

- Before: 7420 lines containing route assembly, job create/events/cancel/approve/input, product turn orchestration, fork, follow-up recovery, project-trust monitoring, attachment cleanup, SSE, and ~1400 lines of tests.
- Approach: split into `jobs.rs`, `turns.rs`, `events.rs` (SSE), `fork.rs`, `monitors.rs` (trust monitoring, attachment cleanup, follow-up recovery), `error.rs` (`ApiError`); `lib.rs` keeps only `router`, `serve*`, and state assembly. Tests follow their modules.
- Mainly for readability and review; limited compile-time benefit, so scheduled last.
- Done criteria: `lib.rs` under 800 lines; move only, no behavior change; `crates/tests/api.rs` all green.

### 5. Daily build habits (no code change)

- Add the target directory and `CARGO_HOME` to the Windows Defender exclusion list (requires administrator rights), or put the repo on a Dev Drive. Done by the repository owner.
- Day to day use `cargo check -p <crate>` and targeted tests; keep `--workspace` for pre-commit and CI.
- If Windows line-table backtraces are unneeded, `[profile.dev] debug` can drop from `line-tables-only` to `0`, saving about 2.2 GB of PDB. This is a team-shared setting and needs its own decision.

## Phase 1 results (2026-10-01, measured on merged main)

| Command | Before | After phase 1 |
| --- | --- | --- |
| `touch crates/engine/src/lib.rs` then `cargo test --workspace --no-run` | 146s (also measured 91s in the same session) | 65–68s |
| Same, cumulative CPU on rove units | 656s, 73 units | 364s, 52 units |
| Rebuild after integration-test-only change | 148s | 40s |
| `cargo check -p rove-api` after api-only change (warm) | ~9s | ~5s |
| `rove_api` lib test unit after api-only change | 26s | 10s |

Slowest units after a runtime change now: `rove_api` lib test 23s, two `rove-desktop` units ~38s combined, `rove_engine` lib test 20s, two integration binaries ~18s each.

## Phase 2: decide after phase 1 lands and is measured

Decision (2026-11-10): split `rove-runtime` is now the committed direction; the blocker documented on 2026-10-01 (module cycles) has been re-measured and is smaller and better understood than first thought. Plan below; superseded 2026-10-01 deferral kept in git history. Items 7–8 remain on-demand.


### 6. Split rove-runtime into multiple crates (complete)

- Re-measured module graph (2026-11-10, `crate::X` cross-dir references, test code excluded):

  ```
  foundation → agents(3 files) · tools(1) · workspace(1) · planning/exec via events
  agents     → context(1: stable_hash) · workspace(3)
  tools      → context(2) · engine(1: delegate turn primitives) · memory(4) · review(2) · state(6) · workspace(3)
  state      → agents(1) · context(1) · foundation(1) · workspace(2)
  planning   → agents(1: procedure types)
  review     → context(1) · tools(2) · workspace(1)
  engine     → everything below it (the orchestrator — expected)
  ```

- **Root cause found:** the "cycles" are almost all *data-type* references, not
  behaviour. `foundation/` is meant to be the base layer, but its serialised
  records (`RuntimeIdentity`, `StreamEvent`) embed types owned by upper layers —
  `AgentProfileIdentity`, `McpServerRuntimeSnapshot`, `ExecutionCapabilities`,
  the planning lifecycle types — so the base ends up depending on its own
  dependants. Three more edges are just calls to one pure helper,
  `context::prompt_metadata::stable_hash`. The single real behaviour cycle is
  `tools/delegate.rs → engine::{run_model_turn, run_tool_turn, ToolTurnContext}`,
  and that is *misplaced turn primitives*: `delegate` already drives
  `rove_core::run_agent_kernel` itself, it only borrows engine's turn helpers —
  it never depends on the `Engine` object.

- **Layer rule adopted (the design fix):** *serialised identity & lifecycle
  data lives at the bottom; behaviour lives above it.* This is the same split
  `rove-protocol` already models (IDs/envelope with no deps). Concretely:
  the base crate owns record/enum shapes; the crates that *produce* them keep
  the logic.

- Target layout (crate names follow the `rove_*` = project_component convention
  used by `rove-core`/`rove-models`/`rove-tools-text`; Bevy-style one-word
  components, no "runtime" infix):

  ```
  rove-foundation   foundation/ + workspace/ + environment identity types
                    + prompt_metadata (stable_hash/tool_signature/workspace_fingerprint)
                    + agents::hashing (content/composite/structured_hash — pure fns)
                    + sunk pure-data LEAF types only: AgentProfileIdentity,
                      AgentDiagnostic, AgentSelector, ProcedureReference·
                      RiskLevel·SideEffect, McpTransport·McpTransportPolicy·
                      McpServerHealthStatus·McpServerRuntimeSnapshot,
                      ExecutionPolicy + planning lifecycle records (StepRecord,
                      PlanRevision, budgets…), CapabilityMutationClass,
                      ExecutionCapabilities/ExecutionEnvironmentIdentity
                    deps: rove-core, rove-models, rove-protocol only  (~8–11k)
  rove-state        state/ + memory/ durable side   deps: foundation, core
  rove-agents       agents/ (activation/validation/package/procedure logic)
                    deps: foundation, core           (its identity types now in foundation)
  rove-tools        tools/ + mcp/ + hooks/          deps: foundation, state, agents, core
  rove-engine       engine/ + planning/ + context/ + review/ + memory session side
                    deps: all of the above + runtime leftovers (conversation, environment behaviour)
  ```

- Naming settled against open-source convention (researched 2026-11-10): `project_component` is the dominant pattern for publishable workspaces (helix `helix-*`, zellij `zellij-*`, goose `goose-*`, bevy `bevy_*`); bare names (`zed`'s `agent`/`editor`, TiKV's `raftstore`) only work when crates are never published. `rove-*` already matches — keep it. `rove-engine` for the orchestrator follows goose's precedent where the core lib wraps its agent loop (agent-inside); the alternative `rove` bare-name core collides with the `rove` CLI binary, so `rove-engine` is the clearer pick.
- Directory convention: once a workspace exceeds ~15 crates the open-source norm is a `crates/` dir (zed, goose, bevy). After the split rove has ~18. Adopt `crates/` for the **library** crates and keep `apps/` for deliverable shells — the same `components/`+`cmd/` split TiKV uses. Move `models`, `core`, `protocol`, `tools-text`, `tests` under `crates/` in the same migration series (each move is its own path-only PR so `git log --follow` history is preserved). **DONE** (`refactor/crates-dir`): all five now live under `crates/`; the workspace is fully `crates/` (libraries) + `apps/` (shells) symmetric.

- Migration order — bottom-up, each an independent measured PR (order corrected 2026-11-10: `state/artifacts.rs::AgentRuntimeArtifacts.agent_profile` embeds `AgentRuntimeProfile`, so `state` depends on `agents` — **agents comes before state**):
  1. `rove-foundation`: move `workspace/`, `foundation/`, `prompt_metadata.rs`; sink the identity/lifecycle data types listed above (pure moves, `pub use` shims left in `rove-runtime` so downstream keeps compiling). Verify `rove-foundation` has zero back-edges. **(PR #65)**
  2. `rove-agents`: move `agents/` logic (activation/validation/package/procedure/selector leftovers). Deps {foundation, core} only — it is the second-lowest layer. `AgentRuntimeProfile` and the `AgentProfileIdentityExt::identity()` conversion live here.
  3. `rove-state`: move `state/` + `memory/` durable side + `foundation/task_state.rs` (TaskState/RunRequest/etc — they embed `AgentRuntimeProfile`, which is why they could not live in foundation). Deps {foundation, agents, core}.
  4. `rove-tools`: move `tools/`+`mcp/`+`hooks/`. The `delegate`↔engine edge is a **two-way** cycle (`tool_turn.rs` builds `SubRunSpawner`; `delegate.rs` drives `run_model_turn`/`run_tool_turn`), and the turn primitives *cannot* sink below `rove-tools` — `tool_turn.rs` is itself the tool-dispatch behaviour (it builds the `Executor`, drives the `ToolRegistry`, constructs the spawner), so moving it down would invert engine↔tools. Break it instead by **splitting `delegate`**: its *data* half (`SubagentRegistry`, `SubUsageLedger`, `SubRunRequest`/`Outcome`/`Error`, `DelegateTool` + `SubRunSpawner` extension reads) has no engine dep and lands in `rove-tools`; its *driver* half (`SubRunSpawner::spawn`, `SubagentKernelHost`, `SubRunApprovalParts` construction — the code that calls `run_model_turn`/`run_tool_turn`) stays in `rove-engine`, reached by `tool_turn.rs` which lives there too. Deps {foundation, agents, state, review, core}.
  5. `rove-engine`: what remains (engine/planning/context) becomes the orchestrator crate — `runtime/` moved to `crates/engine`, `rove-runtime` renamed `rove-engine`, and all compat `pub use` shims deleted; every downstream `rove_runtime::*` import repointed to the owning crate (done, `refactor/rove-tools`).

- De-risk: keep `pub use` re-export shims in `rove-runtime` through the whole migration and only delete them in a final cleanup PR, so `rove-api`/`rove-cli`/`rove-bootstrap`/`rove-desktop` never have to change mid-flight. Update `crates/tests/workspace_architecture.rs` allowlists per PR.

- **Sink only data leaves, never behaviour-bearing types.** `AgentRuntimeProfile` (agents/profile.rs) is a behaviour-bearing aggregate — it embeds `InstructionBundle`, `ProcedureSelection`, `HydratedProcedure`, `ValidationReport` and exposes `allows_capability`/`prompt_slot`/`from_profile`. It stays in `rove-agents`; foundation only needs its *identity projection* `AgentProfileIdentity`. So the conversion direction flips: instead of `AgentProfileIdentity::from_profile(profile)` living on the data type, `rove-agents` gives `AgentRuntimeProfile` an `identity() -> AgentProfileIdentity` method — agents depends on foundation for the leaf type, foundation never hears of the aggregate. Same rule for any other type that pulls a whole cluster: sink the leaf, keep the constructor on the behaviour side.

- Exit criteria (any stage): land only while each split crate compiles standalone with a one-way `Cargo.toml`; measure `cargo check -p <crate>` + `cargo test --workspace --no-run` after a touch in each new crate vs the Phase-1 baseline (65–68s full, 20s `rove_runtime` lib test). If a stage yields no parallelism gain and no dependency-hygiene win, stop and reassess — the goal is a clean acyclic graph and smaller rebuild units, not crates for their own sake.

### 7. Whether to unify the two SQLite stores

- ProductStore (global `product.sqlite`) and runtime StateIndex (per-workspace `state.sqlite`) were deliberately separated on 2026-07-26 (see docs/decisions.md).
- Unifying would change storage format and migration paths — high risk. Revisit only on a concrete problem (e.g. a consistency bug or cross-database query need), and write a decision record first.

### 8. Splitting oversized test files

- `crates/tests/api.rs` (20k lines), `crates/tests/e2e.rs` (8000 lines), `apps/api/src/product/store/tests.rs` (4300 lines) — split into submodules by tested domain. Can be done alongside step 1's test-binary merge.

## Verification

Before and after each step, run once on the same machine and record results:

```bash
touch crates/engine/src/lib.rs && time cargo check -p rove-api -p rove-cli
touch apps/api/src/lib.rs && time cargo check -p rove-api
touch crates/engine/src/lib.rs && time cargo test --workspace --no-run
ls target/debug/deps/*.exe | wc -l
```

Done gate for each step: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` all pass; when api was touched, `/api/openapi.json` is identical to before (attach the diff in the PR).
