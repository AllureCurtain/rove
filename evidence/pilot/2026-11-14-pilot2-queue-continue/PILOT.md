# Pilot 2 — task: continue + queued successor

Pre-registered: 2026-11-14. Session `01M7CTZD8N7FRPA1HF6TRMZV0B`, approval
policy `auto` (switched after the bounded-change run).

- Expectation: while a run is live, send a second message; it must appear in
  the queued strip above the composer and, after the current run finishes,
  start automatically as the next turn.

## Actual

- Sent "Add a power(a, b) function…" → run 3 started (live composer switches
  to 停止).
- While run 3 was live, sent "Also add a modulo(a, b) function…" → it queued
  above the composer with 排队 status and per-message controls
  (编辑 / 立即发送（本轮结束后）/ 插话（打断本轮）/ 撤销该消息).
- Run 3 (`power`) finished `done` with 11,887 events; run 4 (`modulo`, the
  queued message) started automatically as segment 3 and finished `done`
  with 18,412 events — no manual restart needed.

## Verify (independent)

- `git diff` shows both functions landed exactly:
  `power(a, b) → a ** b`, `modulo(a, b) → a % b`, each with unittest cases
  including edge cases (power(0,0), negative operands for modulo).
- `python3 -B -m unittest test_calc`: `Ran 8 tests ... OK`.
- Transcript segments: 4 total, msgs = understand / fix-subtract / power /
  modulo — queue order preserved.

## Verdict

**PASS** — queued successor delivered automatically after the live run, UI
affordances (promote/steer/withdraw) present, both changes landed, suite
green.
