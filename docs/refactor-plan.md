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
- Extracted crates must also respect one-way dependencies, and `tests/workspace_architecture.rs` allowlists get updated.
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
- Done criteria: `lib.rs` under 800 lines; move only, no behavior change; `tests/api.rs` all green.

### 5. Daily build habits (no code change)

- Add the target directory and `CARGO_HOME` to the Windows Defender exclusion list (requires administrator rights), or put the repo on a Dev Drive. Done by the repository owner.
- Day to day use `cargo check -p <crate>` and targeted tests; keep `--workspace` for pre-commit and CI.
- If Windows line-table backtraces are unneeded, `[profile.dev] debug` can drop from `line-tables-only` to `0`, saving about 2.2 GB of PDB. This is a team-shared setting and needs its own decision.

## Phase 1 results (2026-10-01, measured on merged main)

| Command | Before | After phase 1 |
| --- | --- | --- |
| `touch runtime/src/lib.rs` then `cargo test --workspace --no-run` | 146s (also measured 91s in the same session) | 65–68s |
| Same, cumulative CPU on rove units | 656s, 73 units | 364s, 52 units |
| Rebuild after integration-test-only change | 148s | 40s |
| `cargo check -p rove-api` after api-only change (warm) | ~9s | ~5s |
| `rove_api` lib test unit after api-only change | 26s | 10s |

Slowest units after a runtime change now: `rove_api` lib test 23s, two `rove-desktop` units ~38s combined, `rove_runtime` lib test 20s, two integration binaries ~18s each.

## Phase 2: decide after phase 1 lands and is measured

Decision (2026-10-01): do not split runtime for now; rationale in item 6. The remaining phase-2 items become on-demand work.


### 6. Split rove-runtime into multiple crates (deferred)

- Dependency reality: cycles exist between submodules — `foundation↔planning`, `foundation↔tools`, `foundation↔agents`, `foundation↔context`, `foundation↔workspace`, `review↔tools`, `agents↔planning`. To split, the ~30 places where `foundation` references upper layers must first be pushed down or inverted.
- Benefit assessment: downstream bootstrap, api, and cli all depend on tools, state, and agents via `Engine`; after a split, changing those submodules would still force full downstream recompilation — the only savings would be ~20s of runtime-internal parallelism.
- When to revisit: if a submodule (e.g. tools/MCP) needs independent use outside runtime, or if runtime's own compile becomes the dominant wait.

- Candidate split boundaries:
  - `tools` (incl. MCP, 14.5k lines)
  - `agents` (12k lines)
  - `state` (11.5k lines)
  - the rest — engine, planning, context, memory — stays in runtime
- Why deferred:
  - runtime submodule changes are spread out (13–98 each), so the payoff is less certain than for api.
  - Types cross submodule boundaries heavily (engine↔state, tools↔environment); the split cost is high.
- Prerequisite to start: after phase 1, runtime changes are still the dominant wait. Also requires mapping the submodule dependency graph first to confirm no cycles.

### 7. Whether to unify the two SQLite stores

- ProductStore (global `product.sqlite`) and runtime StateIndex (per-workspace `state.sqlite`) were deliberately separated on 2026-07-26 (see docs/decisions.md).
- Unifying would change storage format and migration paths — high risk. Revisit only on a concrete problem (e.g. a consistency bug or cross-database query need), and write a decision record first.

### 8. Splitting oversized test files

- `tests/api.rs` (20k lines), `tests/e2e.rs` (8000 lines), `apps/api/src/product/store/tests.rs` (4300 lines) — split into submodules by tested domain. Can be done alongside step 1's test-binary merge.

## Verification

Before and after each step, run once on the same machine and record results:

```bash
touch runtime/src/lib.rs && time cargo check -p rove-api -p rove-cli
touch apps/api/src/lib.rs && time cargo check -p rove-api
touch runtime/src/lib.rs && time cargo test --workspace --no-run
ls target/debug/deps/*.exe | wc -l
```

Done gate for each step: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` all pass; when api was touched, `/api/openapi.json` is identical to before (attach the diff in the PR).
