# Benchmark Merge Plan

Branch / worktree: `worktree-bench-merge-dynamo-cipher`  
Base commit: `b2045e6`  
Strategy: **Dynamo skeleton + Cipher capabilities + Aegis module structure**  
Sources (read-only):

- Dynamo: `D:/Study/cc/claw/code/CODING_TASK_34_ROVE/res/end/4DSL2P_dynamo_quick`
- Cipher: `D:/Study/cc/claw/code/CODING_TASK_34_ROVE/res/end/4DSL2P_cipher_quick`
- Aegis: `D:/Study/cc/claw/code/CODING_TASK_34_ROVE/res/end/4DSL2P_aegis_quick`
- Basalt: only optional micro-ideas, no bulk copy

Do **not** touch main branch working tree. All edits happen in this worktree.

---

## Goals

1. Replace smoke-only bench with multi-phase, profile-scalable local benchmark.
2. Keep FakeProvider deterministic path (no real API key required).
3. Add HTTP `/bench/*` API matching existing Next proxy style (`/api/x` → backend `/x`).
4. Add real web-ui Benchmark panel (no mock data).
5. Stress config: ≥12 tasks, ≥20 inputs, ≥4 phases, ≥4 check kinds, full evidence per success.
6. Include true cancel+resume (from Cipher), not only scripted tool-failure recovery.
7. Windows-safe command oracles; no Unix-only tools.
8. Keep existing tests green; do not delete tests to pass.

## Non-goals / Do not merge

- Aegis backend routes under `/api/benchmarks/*`
- Basalt stress suite / Windows-broken PowerShell oracles
- Dynamo offline hack that disables `utoipa-swagger-ui`
- Delivery docs (`SUMMARY.md` / `VERIFICATION.md` / `DIFF_SUMMARY.md`) as project assets
- `target/`, `node_modules/`, `.next/`, local `.rove` run garbage

---

## Target architecture

```text
src/bench/
  mod.rs          # re-exports + compat aliases
  schema.rs       # suite/task/check/report/API DTOs
  checks.rs       # check executors (cross-platform)
  runner.rs       # run suite/task, cancel+resume, engine glue
  evidence.rs     # timestamped pack: metrics.json, summary.md, task artifacts
  suites/
    mod.rs
    dataprep.rs   # Dynamo-style parametric generator (default/stress)
    fileforge.rs  # optional Cipher-style heavier generator (or fold into dataprep)
  smoke.rs        # agent-smoke JSON loader / compat

src/interfaces/api/benchmark.rs   # /bench/* handlers + evidence file serve
src/bin/rove-bench.rs             # --suite --profile --output-dir --list

web-ui/
  components/benchmark-panel.tsx  # or rove-bench-panel.tsx
  lib/rove-client.ts              # list/start/get/task/evidence helpers
  lib/rove-types.ts               # bench DTOs
  components/rove-workbench.tsx   # tab entry
  app/globals.css                 # styles
  lib/rove-api-proxy.ts           # Origin fix if chosen
```

API contract (final):

```text
GET  /bench/suites
POST /bench/runs                 # body: { suite, profile, output_dir? }
GET  /bench/runs
GET  /bench/runs/{id}
GET  /bench/runs/{id}/tasks/{task}
GET  /bench/runs/{id}/evidence/{*path}
```

Profiles:

- `default`: short smokeable multi-phase suite (≈4 tasks)
- `stress`: ≥12 tasks, ≥20 inputs, ≥4 phases, ≥4 check kinds

Check kinds (minimum target set):

1. `file_exists`
2. `file_content_contains` (optional exact/regex later)
3. `trace_has_event`
4. `command_oracle` (platform shell; prefer `echo` only)
5. `report_field` (from Aegis/Cipher)
6. `artifact_exists` and/or `report_has_mutation` (from Cipher)

Evidence pack per run:

```text
benchmarks/results/<timestamp>-<suite>-<profile>/
  metrics.json
  summary.md
  result.json            # optional full machine dump
  tasks/<task>/
    workspace/...
    .rove/runs/<run_id>/{trace.jsonl,task_state.json,report.json}
```

Default evidence root is `benchmarks/results/<timestamp>-<suite>-<profile>/`. Inner task runtime state remains under each task workspace `.rove/runs/`.

---

## Phase plan

### Phase 0 — Safety rails

- [x] Create worktree branch `worktree-bench-merge-dynamo-cipher`
- [ ] Confirm base is `b2045e6` and clean
- [ ] Keep sources outside repo; copy only selected files/logic
- [ ] Decide Origin strategy: **A proxy strip Origin (recommended) + B optional localhost defaults for direct browser→API**

### Phase 1 — Module skeleton + schema

**Port from**

| Target | Primary source | Notes |
|---|---|---|
| `src/bench/schema.rs` | Dynamo types + Cipher check enum richness | Keep Dynamo naming closer to current `Benchmark*` if possible for less churn |
| `src/bench/mod.rs` | Aegis/Cipher layout | Re-export public API used by CLI/API/tests |
| delete/replace `src/bench.rs` | — | Replace file with `src/bench/` directory |

**Must preserve / adapt**

- Current public names used by `tests/bench.rs` and `rove-bench` if present: `load_benchmark_suite`, `run_benchmark_suite`, `BenchmarkSuite`, etc.
- `agent-smoke.json` still loadable (compat layer or schema migration with tests)

**Acceptance**

- `cargo check` / unit tests for schema deserialize compile

### Phase 2 — Checks + evidence

| Target | Source | Take |
|---|---|---|
| `src/bench/checks.rs` | Cipher + Aegis | 6+ check kinds; Windows/Unix shell branch like Dynamo/Cipher (`powershell`/`sh`), no `wc/ls/grep/test` |
| `src/bench/evidence.rs` | Dynamo + Cipher | metrics.json + summary.md + per-task paths; load helpers for API |

**Acceptance**

- Unit tests for each check kind with temp dirs
- Evidence writer creates non-colliding timestamp dirs

### Phase 3 — Runner + suites

| Target | Source | Take |
|---|---|---|
| `src/bench/runner.rs` | Dynamo baseline run loop | workspace setup, FakeModelClient turns, collect artifacts |
| cancel+resume path | **Cipher** `run_task_with_cancel_resume` + `merge_run_reports` | true mid-task interrupt |
| suite generator | Dynamo `generate_dataprep_suite` first | parametric default/stress |
| optional heavier generator | Cipher `fileforge` ideas | more inputs/phases/failures if dataprep too template-y |
| `src/bin/rove-bench.rs` | Dynamo CLI shape | `--suite --profile --output-dir --list` |

**Acceptance**

- CLI default: all tasks pass on Windows
- CLI stress: ≥12/≥20/≥4/≥4, all pass on Windows with FakeProvider
- At least one stress task exercises cancel+resume and still produces merged evidence

### Phase 4 — HTTP API

| Target | Source | Take |
|---|---|---|
| `src/interfaces/api/benchmark.rs` | **Dynamo** | 6 routes incl. evidence file read + canonicalize traversal guard |
| wire into `api/mod.rs` | Dynamo | `BenchState` on `ApiStateInner`, router merge |
| DTOs | Dynamo `types.rs` / Cipher schema API types | keep response fields stable for UI |
| OpenAPI tags | Aegis/Cipher docs helpers if cheap | do **not** remove swagger-ui |

**Acceptance**

- curl/Invoke-WebRequest: list suites, start run, poll status, task detail, evidence file
- paths are `/bench/...` not `/api/benchmarks/...`

### Phase 5 — Origin / proxy fix

Recommended:

1. In `web-ui/lib/rove-api-proxy.ts`, drop browser `origin` (and maybe `referer`) when forwarding to Rust API, because this is a server-side proxy hop.
2. Optionally document `ROVE_CORS_ORIGINS=http://localhost:3000,http://127.0.0.1:3000` for direct API access.
3. Add/adjust proxy unit test asserting Origin is not forwarded.

**Do not** silently weaken production auth/token rules.

### Phase 6 — Web UI

| Target | Source | Take |
|---|---|---|
| `components/benchmark-panel.tsx` | Dynamo panel as base | real polling loop that worked in review |
| UI polish | Aegis/Cipher panels | expandable checks, failure reasons, evidence links |
| `lib/rove-client.ts` / `rove-types.ts` | Dynamo | `/api/bench/...` client paths |
| workbench tab entry | Dynamo/Aegis | Benchmarks tab beside Agent |
| CSS | any of the three | keep design tokens consistent with workbench |

**Acceptance**

- `pnpm build` / tsc pass
- With API up: list suite, switch profile, start run, see results + evidence paths
- No static mock benchmark data

### Phase 7 — Tests & verification

- Update `tests/bench.rs` for new schema/profiles
- Add API tests for `/bench/*`
- Keep existing API/e2e tests green
- Real commands on Windows:
  - `cargo fmt --all -- --check`
  - `cargo build --all-targets`
  - `cargo test`
  - default bench
  - stress bench
  - API smoke
  - web-ui build
  - optional manual UI loop

### Phase 8 — Cleanup before any PR

- No secrets
- No `target/`, `node_modules/`, `.next/`
- No personal `.rove` run state unless tiny fixture intentionally committed
- Prefer not committing large evidence dumps; gitignore generated results if needed

---

## File-level merge checklist

### Create

- [ ] `src/bench/mod.rs`
- [ ] `src/bench/schema.rs`
- [ ] `src/bench/checks.rs`
- [ ] `src/bench/runner.rs`
- [ ] `src/bench/evidence.rs`
- [ ] `src/bench/suites/mod.rs`
- [ ] `src/bench/suites/dataprep.rs`
- [ ] `src/interfaces/api/benchmark.rs`
- [ ] `web-ui/components/benchmark-panel.tsx`

### Replace / heavily rewrite

- [ ] `src/bench.rs` → remove after directory module lands
- [ ] `src/bin/rove-bench.rs`
- [ ] `tests/bench.rs` (adapt, do not delete coverage)

### Modify

- [ ] `src/lib.rs` (already `pub mod bench`; ensure path works with directory)
- [ ] `src/interfaces/api/mod.rs` (router + state)
- [ ] `src/interfaces/api/types.rs` (DTOs) and/or keep DTOs in benchmark module
- [ ] `src/interfaces/api/docs.rs` (tag only if useful)
- [ ] `web-ui/lib/rove-client.ts`
- [ ] `web-ui/lib/rove-types.ts`
- [ ] `web-ui/lib/rove-api-proxy.ts` (+ test)
- [ ] `web-ui/components/rove-workbench.tsx` or `app/page.tsx` tab entry
- [ ] `web-ui/app/globals.css`
- [ ] `benchmarks/agent-smoke.json` only if schema requires migration
- [ ] `.gitignore` for bench result dumps if needed
- [ ] `Cargo.toml` only if extra dep truly required (e.g. regex). **Do not** drop swagger-ui.

### Optional later

- [ ] Cipher `fileforge` as second suite
- [ ] Basalt `RecorderStats` trait if private field access becomes painful
- [ ] Aegis richer `report_field` conditions (gte/lte/contains)

### Never copy as-is

- [ ] Aegis `/api/benchmarks/*` route table
- [ ] Basalt resume PowerShell `Select-String` oracles
- [ ] Dynamo swagger-ui disable patch
- [ ] Any model’s root `SUMMARY.md` / `VERIFICATION.md` / `DIFF_SUMMARY.md`

---

## Risk register

| Risk | Mitigation |
|---|---|
| Schema break of `agent-smoke` / old tests | Compat loader + dedicated tests first |
| Origin 403 blocks UI | Proxy strip Origin + explicit docs |
| Windows shell oracle flakes | Only `echo`-class commands; prefer Rust checks for files |
| Cancel+resume flaky event loss | Use Cipher deferred-cancel after tool completion; merge traces |
| Giant single-file regression | Force `src/bench/` split from day one |
| Accidental main-branch edits | Stay in this worktree only |
| Over-merging three codebases → frankenstein API | Freeze API contract above; implement once |

---

## Implementation order (when coding starts)

1. Schema + checks + evidence (no API yet)
2. Runner + dataprep default/stress + CLI
3. Real Windows CLI verification
4. HTTP API + evidence read
5. Proxy Origin fix
6. Web UI panel + wiring
7. Full verification matrix
8. Commit on worktree branch (only when asked)

---

## Definition of done

- [ ] Default + stress pass on Windows with FakeProvider
- [ ] Stress meets numeric floors
- [ ] True cancel+resume task present and green
- [ ] `/bench/*` API complete including evidence file read
- [ ] Web UI real end-to-end against running API
- [ ] `cargo fmt/test/build` green; web-ui build green
- [ ] No secrets / build artifacts committed
- [ ] Existing non-bench flows still work


## Implementation status (worktree)

Completed in this worktree on 2026-07-12:

- Dynamo-based `src/bench.rs` with dataprep default/stress + agent-smoke compatibility
- Evidence packages under `benchmarks/results/<stamp>-<suite>-<profile>/`
- HTTP `/bench/*` API with evidence file read
- Proxy strips browser `Origin`/`Referer`
- Web-ui Benchmarks tab + panel
- Verified: cargo check, bench tests, dataprep default 4/4, stress 14/14, API smoke, vitest proxy, tsc, next build

Deferred / follow-ups:

- Split `src/bench.rs` into `src/bench/` modules (Aegis structure polish)
- Cipher true cancel+resume task (scripted recoverable failure is present; mid-run cancel not yet)
- Optional richer Cipher check kinds beyond the current four+legacy checks

### Phase 2 completed (2026-07-12)

- Split into `src/bench/{mod,schema,checks,runner,evidence,suites}.rs`
- Added Cipher-style cancel+resume on stress last task (`resumed=true`)
- Added `report_field` + `artifact_exists` checks
- Verified: bench tests 7/7, default 4/4, stress 14/14 with resume task

