# Pilot 2 — task: long history

Pre-registered: 2026-11-14. Main session has 8+ runs totalling >60k
projected events.

- Expectation: transcript endpoint returns the session with explicit
  `partial_reasons` where truncated; new run still receives the first-turn
  context in its model history.

## Actual

- `GET /transcript` on the main session: 7 segments, **60,000 events**,
  `status: partial` with a typed reason
  `response_limit_reached { run_ordinal: 7, expected_seq: 363,
  observed_seq: 2941 }` — the error run's tail is projection-truncated, not
  lost (the underlying trace is complete; the UI surfaces the terminal
  state from job state regardless).
- Sent "Without re-reading files: what was the bug in subtract() and what
  is the file path?" → run 9. Its `task_state.json` history carried **221
  entries**, first entry = the verbatim first user message; the serialized
  history contains the `subtract` bug and `calc.py` — so the first-turn
  context genuinely reached the model.
- The model misread the intent and probed the durable-memory tool
  (`reindex_memory` + 12 `read_memory` name guesses) instead of answering
  from context — same over-verification behavior noted before. Run still
  finished `done`/`final` (9,806 events) without stalling the session.

## Verify (independent)

- `task_state.json` history[0] verbatim = "In this repo, find where
  subtraction is implemented…" and the history JSON contains both
  "adds instead" (the bug) and "calc.py".
- Transcript `partial_reasons` is typed and names the exact run + seq gap —
  no silent truncation.

## Verdict

**PASS** — long history is bounded and explicit; context delivery verified
at the model-history level. Model-side tool-choice observation noted for
the report (memory probe instead of in-context recall).
