# Real engineering pilot — summary report

**Scope:** supervised real-provider engineering trial of the rove web console, per `docs/engineering-validation.md` task matrix. This is the record that the deterministic fake model does **not** provide: a credentialed real provider (DeepSeek) driving real tasks through the actual UI.

- **Date:** 2026-11-14 · **Operator:** rove product driven by the ego-lite browser (real Chromium, real pointer/keyboard); tasks dispatched and evidence reconciled by a supervising agent.
- **Build under test:** rove `main` — runs progressed across `d8aec71` → `6693bbb` as defects were fixed in-flight (each run records its own commit).
- **Provider:** DeepSeek (`https://api.deepseek.com`) — `deepseek-v4-pro` for early smoke runs, `deepseek-flash` for the pilot matrix (operator-chosen lower-cost model).
- **Surfaces:** web console served by `rove-api --web-dist` on loopback, isolated `ROVE_CONFIG_ROOT`/`ROVE_DATA_ROOT` per run so no real user state was touched.

## Matrix results

| Task-matrix row | Session | Verdict | Evidence |
| --- | --- | --- | --- |
| First launch | smoke-0 / smoke-1 | GO after fix | `2026-11-14-smoke-1/` |
| Understand code | smoke-1 | PASS | `2026-11-14-smoke-1/` |
| Make a bounded change | s2-replay | PASS (heavy operator lift → fixed) | `2026-11-14-s2-replay/` |
| Approval (approve) | s3-flash, s4, s5 | PASS | respective dirs |
| Approval (deny) | s6-failures | PASS — fail-closed, write absent | `2026-11-14-s6-failures/` |
| Continue (2nd task + queue) | s3-flash, s4 | PASS | `2026-11-14-s3-flash/` |
| Cancel mid-run | s4 | PASS — terminal `cancelled`, partial kept, new task runs | `2026-11-14-s4-runtime-control/` |
| Refresh / restart recovery | s4 | PASS — run + pending approval survive reload | `2026-11-14-s4-runtime-control/` |
| Duplicate launch | duplicate-launch | PASS — second host refused by ownership lock | `2026-11-14-duplicate-launch/` |
| Inspect + revert | s5-revert | PASS — byte-exact restore, 409/404 boundaries | `2026-11-14-s5-revert/` |
| Isolate (workspaces + fork) | s4 | PASS — per-workspace trust, independent fork | `2026-11-14-s4-runtime-control/` |
| Long history | s4 | PASS — stable position, paged loads | `2026-11-14-s4-runtime-control/` |
| Failure paths | s6-failures | PASS — typed errors, no fabricated success, no leaked secrets | `2026-11-14-s6-failures/` |

Every scenario directory holds `PILOT.md` (pre-registered expected outcome, actual, independent verification, verdict), `notes.md`, session exports, and screenshots. Runs that failed are preserved, not deleted.

## What the pilot proved

- **Real tasks complete end-to-end.** A real provider answers code-understanding questions with file:line citations verified exact against the repo (9/9 on smoke-1; every citation on the s3/s4 turns).
- **A bounded change is diagnosed and applied.** On s2, the agent located the per-run transcript event cap and produced a fix **identical in file, constants, values (20000/25000) and intent** to the real upstream commit `31cbfdb`, compiled clean, with no unrelated churn.
- **The control surface holds under interruption.** Cancel leaves a terminal `cancelled` record with partial output preserved; a browser refresh restores a live run *and* its pending approval; a second host on the same data root is refused by the ownership lock; revert restores byte-exact originals with correct conflict/not-found refusals; fork is correctly gated to cleanly-completed turns.
- **Failures fail safe.** Invalid credentials, dead endpoints, non-existent models, missing files, and denied approvals all surface typed, actionable errors — no fabricated success, no leaked secrets, no silent side effects.

## Defects the pilot exposed (and their disposition)

The pilot's primary engineering value was exposing real defects that the fake-model harness could not:

| # | Defect | Disposition |
| --- | --- | --- |
| 1 | `rove_bin()` derived the CLI path from `current_exe`, breaking under the shared/scratch target layout — 10 `cli_repl`/`cli_review` integration tests panicked on CI's cold-cache path | **Fixed** — `3deb4cd` reads the path from `cargo build --message-format=json` artifact JSON |
| 2 | `DEFAULT_MAX_MODEL_TURNS_PER_STEP = 4` killed every real tool-using run before synthesis | **Fixed** — `717b156` raised to 16, then `6693bbb` to 48 after s3 showed 16 still starved an exploratory step |
| 3 | `context_hard_limit_tokens = 30_000` (fake-model-sized) and `runtime.execution.*` had no operator surface | **Fixed** — `5190b6b` raised to 128k + added `[runtime.execution]` in user config + 12 `ROVE_EXECUTION_*` env vars |
| 4 | Provider "Verify and save" wiped the key when model validation failed | **Fixed** — `42b1b44` restores the key on a failed verify |
| 5 | Transcript projection drops tool-call cards (incl. the rollback affordance) at the ~2000-event cap — reproduced in 4 sessions | **Open** — filed as issue #80; a chatty model crosses the projection window reliably |
| 6 | Stranded-queue `needs_attention` deadlock requires a manual recover click on every failed turn | **Open (design)** — conservative by intent; follow-up proposed: one-click "recover and continue" for failed+drained runs |
| 7 | `deepseek-flash` over-explores on read-heavy tasks (chained `run_shell` approvals, high token burn) | **Noted** — model behavior, not a rove defect; informs model choice + task sizing |

## Honest limits

- **Token cost:** `deepseek-flash` is verbose — single turns burned 0.5–1.7M tokens on read/explain tasks. Real-provider evaluation is not free; totals are per-run in each `PILOT.md`.
- **One provider, one machine:** results are on macOS arm64 + DeepSeek. They establish *a* real-provider path works, not that all providers/models behave alike.
- **Budget defaults were tuned in-flight:** the "out of the box" PASS was reached only after the baseline fixes (#78/#79) landed; earlier runs needed manual rescue — that history is kept, not smoothed over.
- **Not covered:** signed/clean-machine Desktop, third-party MCP interop, code signing, and a *second* real project beyond rove's own repo.

## Bottom line

The supervised pilot establishes that rove's web console, on a credentialed real provider, completes real engineering tasks — understand, change, approve, interrupt, recover, revert, isolate — with byte-exact and fail-closed behavior, and that the exercise surfaced and fixed real product defects. That is the "validated in actual engineering practice" evidence the degree criterion pairs with the software copyright.
