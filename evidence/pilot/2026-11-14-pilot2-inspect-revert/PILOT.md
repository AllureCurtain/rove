# Pilot 2 — task: inspect change + byte-exact revert

Pre-registered: 2026-11-14.

- Expectation: revert the latest recorded mutation on `calc.py` via
  `POST /sessions/{id}/mutations/revert`; bytes return to the pre-mutation
  state.

## Actual

- Work panel → 变更 tab lists `calc.py` `update` (all session edits
  aggregated per file). Opened the file-preview tab to inspect the current
  content (the refactored docstring/type-hint version, sha256:38749f3f…).
- Called `POST /mutations/revert {"path":"calc.py"}` →
  `{reverted_operation: "update", run_id: "01M7CVVDTY3KB3MBHKCDC28EA8",
   action: "reverse_patched"}` — the run-6 refactor mutation was selected
  (newest binding first) and reverse-applied.

## Verify (independent)

- `head -15 calc.py` after revert: plain `def add(a, b): return a + b` —
  docstrings and annotations gone; `subtract` fix, `power`, `modulo` all
  still present (those were earlier mutations, untouched).
- `python3 -B -m unittest test_calc`: `Ran 8 tests ... OK`.
- Revert correctness is byte-exact by construction: `revert_update` requires
  current bytes == recorded `after_version`, reverse-patches the recorded
  diff, and refuses to publish unless the result hashes to `before_version`.

## Verdict

**PASS** — byte-exact revert of the most recent mutation via the recorded
diff, scoped to that one mutation; earlier edits preserved.
