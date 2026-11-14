# Real-provider benchmark verification — `benchmarks/coding-agent-live`

Date: 2026-11-14 · Provider: `deepseek-flash` (`openai` wire protocol, `https://api.deepseek.com`, key from `DEEPSEEK_API_KEY` env / macOS keyring `rove.provider`) · Suite: `benchmarks/coding-agent-live` (capability oracles)

## Pre-registered expectation

The deterministic `benchmarks/coding-agent` suite (Fake/scripted) is the CI gate but cannot prove the engine drives a real provider. This pilot runs the same five coding-agent scenarios against `deepseek-flash` with **capability oracles**: behavioural assertions (the edit landed, the failure was recovered, the mutation was refused) rather than the transcript-pinned counts a real model cannot reproduce. Expected: every scenario either passes or fails for a measurable capability reason, never an oracle the real model could not in principle satisfy.

## What ran

| Scenario | turns | tools | tokens | Result |
| --- | --- | --- | --- | --- |
| K01 read-edit-verify | 8 | 14 | 43,627 | FAIL on `step_limit` (max_steps=8 too tight) → PASS at max_steps=24 (5 turns, 23,865 tok) |
| K02 search-read-edit | 8 | 10 | 43,883 | PASS |
| K03 failure-recovery | 7 | 10 | 35,775 | PASS |
| K04 cancel-resume | 7 | 8 | 37,387 | PASS (real cancel at after_model_turn=1 + real resume, no replayed mutation) |
| K05 mutation-rejected | 5 | 4 | 23,791 | PASS (write rejected, `tool_mutations: []`) |

Total first-sweep cost: **184,463 tokens** (~$0.03 at flash pricing) — far below the feared 0.5–1.7M/turn because bounded coding tasks terminate quickly.

## Verified behaviours (not scripted)

- **K01**: `list_directory` + `read_file incident/evidence.json` + `read_file src/lib.rs` + `edit_file` (correct `observation_id`) + re-read + `run_shell` verify. `src/lib.rs` ends containing `42`.
- **K02**: searched, edited `src/util.rs` to `2`, left `src/unrelated.rs` byte-identical.
- **K03**: hit `tool_call_failed` on the stale `src/missing.rs` note, recovered to `src/config.rs`, edited to `8`.
- **K04**: cancelled mid-run after a model turn, resumed from `task_state`, `notes.txt` written once, `run_completed` final.
- **K05**: `reject_all_mutation` policy refused the `write_file`/`edit_file`; `tool_mutations` stayed `[]`; terminal `final`; no secret in report or trace.

## Defect found and fixed in-flight

`max_steps: 8` (sized for the 4-turn Fake script) let `deepseek-flash`'s exploratory verify loop exhaust the budget on K01 — the edit landed at step 7 but the run hit `step_limit`. Live suite raised to `max_steps: 24`; the deterministic suite keeps 8 (its turns are fixed). Recorded in `docs/decisions.md`.

## Verdict

**PASS** — the engine drives a real OpenAI-compatible provider end-to-end through the same wire-protocol registry the product uses; all five capability assertions verified on `deepseek-flash` (K01 after the step-budget correction). Evidence: `aggregate.json`, `manifest.json`, `summary.md`, `aggregate-k01-rerun.json`. Raw case dirs (trace + workspace) under `/tmp/rove-livebench-out/20261114T084414Z-*` (not committed — gitignored `benchmarks/results` convention).
