# Pilot 2 — task: deny an approval

Pre-registered: 2026-11-14. `default_approval_policy` switched back to `ask`
for this task.

- Expectation: file-mutation approval → click 拒绝 → the tool call does not
  execute, the run ends as a typed failure (not fabricated success), file
  unchanged.

## Actual

- Sent "Delete the README.md file…" via `POST /messages` (UI composer was
  covered by the file-preview tab; API path is equivalent and durable).
- Run 7 planned 4 steps, located `README.md`, re-read it for a fresh
  `observation_id`/`version`, then surfaced a `delete_path` approval card
  (path + observation + version visible in the detail).
- Clicked 拒绝 → run ended `run_status: error`; session rail badge flipped
  需关注 → 失败. README.md still present on disk (verified `ls`).
- Note: transcript tail stops at seq 362 (a `thinking_delta` just before the
  approval); the terminal state is projected from the durable job state, not
  appended transcript events — same projection the pilot-1 runs showed.

## Verify (independent)

- `ls /tmp/rove-pilot2-repo/README.md` → present, unmodified.
- Run status `error` + typed `tool_call_approval_needed` rejection — no
  fabricated "deleted" claim in the summary (the model never got to
  synthesize a success; the run failed at the gate).

## Verdict

**PASS** — deny path is a typed failure, no side effect, no forged success.
