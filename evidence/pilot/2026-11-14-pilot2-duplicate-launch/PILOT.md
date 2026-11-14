# Pilot 2 — task: duplicate-launch refusal

Pre-registered: 2026-11-14.

- Expectation: a second `rove-api` on the same data root is refused
  (host ownership), even on a different port.

## Actual

- Live API on pid 43490 (`ROVE_DATA_ROOT=/tmp/rove-pilot2-data`).
- Launched a second `rove-api` on port 18801 with the same
  `ROVE_CONFIG_ROOT`/`ROVE_DATA_ROOT` → exited immediately with:
  `Error: Another Rove instance is already using this data directory.
   Close it before starting another instance.`
- First API unaffected — still serving on 18793.

## Verify (independent)

- `curl http://127.0.0.1:18793/api/product/preferences` after the refused
  launch → 200, unchanged.

## Verdict

**PASS** — data-directory ownership is enforced process-wide; a different
port does not permit sharing.
