# Pilot 2 — task: typed failure paths

Pre-registered: 2026-11-14.

- Expectation: each failure is a typed error — no fabricated success, no
  leaked secret.

## Actual

**(a) Nonexistent model.** Session `01M7CWZR99Y9611MJ8NZ6M4NPC` on ws2 with
`model: deepseek-nonexistent-9000`. The first queued follow-up exhausted 4
start attempts with `project_trust_required` (the new session's model config
had again invalidated `provider_credentials` — the trust-digest trap);
after re-grant + control confirm, the run executed and terminated
`reason: error`, `outcome: failed` with the verbatim provider detail:
`HTTP 400 Bad Request: {"error":{"message":"The supported API model names
are deepseek-flash, deepseek-v4-pro, but you passed
deepseek-nonexistent-9000.","type":"invalid_request_error"}}` — typed,
truthful, and the API key appears nowhere in the event stream or output.

**(b) Missing file.** Covered by the ws2 isolation run: `read_file calc.py`
→ `Error: workspace file was not found`; the model reported absence plainly
rather than inventing contents.

**(c) Denied approval.** Covered above: `delete_path` denied → run
`run_status: error`, `README.md` untouched, no fabricated success.

## Verify (independent)

- `report.json` for the bad-model run: `termination_reason: error`,
  `outcome: failed` (not success); the surfaced text quotes the provider's
  own message and names only public model identifiers.
- `grep -i 'sk-'` over the run's `trace.jsonl` → 0 hits (secret never
  entered the event record).

## Verdict

**PASS** — all three failure classes produce typed, truthful errors with no
fabrication and no secret leakage.
