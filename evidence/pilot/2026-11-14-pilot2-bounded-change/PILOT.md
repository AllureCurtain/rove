# Pilot 2 — task: bounded change (fix subtract bug + run tests)

Pre-registered: 2026-11-14. Same scratch repo; continuing session
`01M7CTZD8N7FRPA1HF6TRMZV0B` (deepseek-flash, max_steps 24).

- Expectation: agent fixes `calc.py::subtract` to `return a - b`, runs the
  test suite, all 4 tests pass. Only `calc.py` changes; diff is the one-line
  fix.

## Actual

- Second run in the session, status `done`, 13,026 events.
- Plan of 5 steps. The `edit_file` call carried the exact expected payload:
  `old_text` = the two buggy lines (comment + `return a + b`), `new_text` =
  `    return a - b`, plus `version`/`observation_id` byte-identity guards.
- **8 sequential tool approvals** were required in this single run — one
  `edit_file`, seven `run_shell` verification commands (the model kept
  re-combining sha256sum + unittest + git status + `__pycache__` inspection
  instead of trusting earlier evidence). Each surfaced as a blocking approval
  card; run resumed on every approve. This is the same over-verification
  pattern as pilot-1 defect #7, amplified under `Ask` policy.

## Verify (independent)

- `git diff calc.py`:

  ```diff
   def subtract(a, b):
  -    # Deliberate defect for the pilot: subtraction should be a - b.
  -    return a + b
  +    return a - b
  ```

  Exactly the bounded change; comment removed too (acceptable — the comment
  described the defect, not the contract).
- `python3 -B -m unittest test_calc`: `Ran 4 tests ... OK`.
- `git status --porcelain`: only `?? __pycache__/` (pre-existing from run 1).

## Verdict

**PASS** — bounded change landed byte-exact, suite green, approval gate
exercised under real load (8×). Note for the report: `deepseek-flash` under
`Ask` generates heavy approval friction on verification loops; for longer
matrix tasks the session's approval policy was moved to `auto` after this run
so a single over-cautious model cannot stall the matrix — the reject path is
still covered by the dedicated deny test below.
