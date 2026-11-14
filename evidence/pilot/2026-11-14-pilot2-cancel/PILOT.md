# Pilot 2 — task: cancel a live run

Pre-registered: 2026-11-14. Same session.

- Expectation: start a run, click 停止 mid-flight; the run ends cancelled
  (not done), composer returns to send state, no new file mutations land
  after the stop.

## Actual

- Sent "Refactor calc.py so every function has a docstring and type hints…"
  → run 5 started, plan of 6 steps.
- Clicked 停止 ~12s in while the model was still on step 1 (inventorying
  `calc.py`). Composer immediately returned to 发送; no confirm dialog on
  this build (stop is single-click).
- Run status after stop: `cancelled`, 516 events, terminal
  `run_completed { reason: "cancelled" }` with finalization record
  `outcome: "cancelled", mode: "deterministic"`.

## Verify (independent)

- `git diff calc.py` relative to HEAD shows only the earlier committed work
  (subtract fix + power/modulo) — the cancelled refactor added zero lines.
- `python3 -B -m unittest test_calc`: still `Ran 8 tests ... OK`.
- Transcript tail: `execution_budget_updated` → `finalization_completed
  (cancelled)` → `run_completed (cancelled)` — clean terminal ordering, no
  dangling tool calls.

## Verdict

**PASS** — cancel propagated end-to-end (UI → supervisor → finalization →
terminal event), no post-stop mutation, composer restored for the next turn.
