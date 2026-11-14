# Pilot — duplicate-host launch (ownership lock)

- **Date**: 2026-11-14 · **Operator**: direct bash (no browser needed — process-level check)
- **Env**: rove `main` @ `6693bbb`, `target/debug/rove-api`, isolated `ROVE_CONFIG_ROOT=/tmp/rove-dup-config`, `ROVE_DATA_ROOT=/tmp/rove-dup-data`, `TMPDIR=/private/tmp`.

## Scenario
Start one `rove-api` on a data root, then start a second `rove-api` on the SAME data root (different port). Expected: the second host is refused before it can interfere with the first's recovery/state — the ProductStore directory is owned by an exclusive OS lock (see decisions "API data ownership precedes startup recovery").

## Actual
- First instance bound 127.0.0.1:18795, `/health` → `{"status":"ok"}`.
- Second instance on same `ROVE_DATA_ROOT` (port 18796) exited immediately:
  `Error: Another Rove instance is already using this data directory. Close it before starting another instance.`
- First instance remained alive and serving after the refusal.

## Verdict: PASS
Duplicate host on the same data root is refused cleanly at startup; the owner is unaffected. Matches the ownership-lock decision and the acceptance matrix's duplicate-launch row.

## Evidence
Command transcript above (first health check, second refusal text, P1 liveness).
