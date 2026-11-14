# Pilot — issue #80 transcript projection fix, real-provider verification

**Pre-registered expected outcome** (written before the run): a real `deepseek-flash` turn that produces more than 2,000 indexed canonical events must be projected whole by `GET /product/sessions/{id}/transcript` — no `response_limit_reached` partial reason, and the late tool-call cards (incl. `edit_file`, which carries the rollback affordance) present in the segment. Under the previous `MAX_SNAPSHOT_EVENTS = 2_000` index clamp this run would have returned at most 2,000 rows and reported `response_limit_reached` mid-run.

**Build under test:** `rove-api` on `main` `4c8827d` (the PR #82 fix, `MAX_SNAPSHOT_EVENTS` 2000→50000, `MAX_EVENTS_PER_RUN` 20000→50000, `MAX_TOTAL_EVENTS` 25000→60000).

**Setup:** isolated `ROVE_CONFIG_ROOT`/`ROVE_DATA_ROOT` under `/private/tmp`, scratch workspace `/private/tmp/rove-i80-ws`, provider `deepseek-flash` (OpenAI-compat) onboarded to keyring, `provider_credentials` + `project_configuration` + `workspace_instructions` trust granted, session `model-config` set to the flash profile.

**Actual:** a single turn asking the agent to edit `execution.rs` (`DEFAULT_MAX_MODEL_TURNS_PER_STEP` 48→49) produced **4042 indexed canonical events** — dominated by `thinking_delta` (2758) and `llm_chunk` (1230), exactly the event classes that crossed the old 2,000 clamp.

| Check | Result |
| --- | --- |
| Projected events | **4042**, `observed_through_seq` 4042, `partial_reasons: []` — no truncation |
| `response_limit_reached` | **absent** (was the symptom in 4 prior sessions) |
| Tool-call cards present | `glob_paths`, `search_code`, `read_file`×2, `edit_file`×2 — incl. the rollback card |
| `thinking_delta`/`llm_chunk` rows | 2758 + 1230 projected (thinking block + answer text rebuild) |
| `status` | `complete` |

**Independent verification:** counted event types + tool cards from the live `transcript` JSON; confirmed `edit_file` started/completed at seq 4040–4042 (the tail of the run) — the exact region the old clamp dropped.

**Verdict: PASS.** The #80 fix holds on a real chatty-provider run; the projection no longer truncates at 2000 and the rollback card survives.

**Artifacts:** `transcript-final.json` (the full projected transcript, 4042 events).

**Honest note:** the turn was still `running` at capture (the model emits a large report after the edit); the fix's observable — full projection past 2000 with tool cards — is independent of the final terminal event. Setup surfaced two operational traps (recorded, not defects): a session created before `provider_selection` retains `model:"fake"` (fixed via `model-config` PUT), and a profile-id change re-triggered `provider_credentials` trust (re-granted).
