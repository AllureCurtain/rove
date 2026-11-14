# Pilot 2 — task: refresh / restart recovery

Pre-registered: 2026-11-14. Same session; cancelled run was the latest segment.

- Expectation: page reload restores transcript; `rove-api` restart keeps
  workspace + trust + provider + session; a new turn starts normally after.

## Actual

- **Page reload** (API still up): transcript restored — all 5 turns listed,
  latest run rendered as "执行已停止，部分工作可能尚未完成" (cancelled
  summary) with expandable execution details.
- **API restart**: killed pid 39227, relaunched with the same
  `ROVE_CONFIG_ROOT`/`ROVE_DATA_ROOT` (new pid 43490). After restart:
  `GET trust` → `trusted`; `GET transcript` → 5 segments, status complete.
- Browser reload against the new process rendered the same session; sent
  "What does modulo(-7, 3) return…" → run 6 executed live and finished
  `done` (9,361 events; the model answered `-7 % 3 == 2` under Python's
  floored-division modulo).

## Verify (independent)

- `curl /trust` and `/transcript` post-restart return identical durable
  state (identity_digest unchanged, all segments intact).
- New run completed end-to-end on the restarted host — no re-onboarding,
  no trust re-grant, no credential re-entry.

## Verdict

**PASS** — refresh restores the transcript faithfully including the
cancelled turn; full API restart preserves trust/session/provider state and
the next turn starts cleanly.
