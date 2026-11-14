# Pilot 2 report — second real-project pilot (scratch repo)

Date: 2026-11-14. Supervised `deepseek-flash` trial of the Web console on a
disposable repo (`/tmp/rove-pilot2-repo`, Python calc module with a
deliberate `subtract` bug) — proving the runtime contract on a project that
is *not* rove's own tree.

- Host: `rove-api` 127.0.0.1:18793, `ROVE_CONFIG_ROOT=/tmp/rove-pilot2-config`,
  `ROVE_DATA_ROOT=/tmp/rove-pilot2-data` (isolated; real `~/.rove` untouched).
- Provider profile `01M7CTYT30RQZVTXS62SKPT1AK` (`deepseek-flash`, keyring).
- Workspaces: `pilot2-scratch` (repo) + `pilot2-other` (folder, isolation).
- Sessions: `Pilot 2 main` (9 runs), `Pilot 2 other` (1), `Pilot 2 fork` (1),
  `Pilot 2 badmodel` (1).

## Matrix

| # | Scenario | Verdict | Evidence |
|---|---|---|---|
| 1 | Understand code | **PASS** | `2026-11-14-pilot2-understand/` |
| 2 | Bounded change (fix + tests) | **PASS** | `2026-11-14-pilot2-bounded-change/` |
| 3 | Approve (many ×) | **PASS** | same — 8 sequential approvals exercised |
| 4 | Continue + queued successor | **PASS** | `2026-11-14-pilot2-queue-continue/` |
| 5 | Cancel mid-run | **PASS** | `2026-11-14-pilot2-cancel/` |
| 6 | Refresh / restart recovery | **PASS** | `2026-11-14-pilot2-refresh-recovery/` |
| 7 | Duplicate-launch refusal | **PASS** | `2026-11-14-pilot2-duplicate-launch/` |
| 8 | Inspect + byte-exact revert | **PASS** | `2026-11-14-pilot2-inspect-revert/` |
| 9 | Deny approval | **PASS** | `2026-11-14-pilot2-deny-approval/` |
| 10 | Workspace + fork isolation | **PASS** | `2026-11-14-pilot2-workspace-fork/` |
| 11 | Long history | **PASS** | `2026-11-14-pilot2-long-history/` |
| 12 | Typed failure paths | **PASS** | `2026-11-14-pilot2-failure-paths/` |

## Findings

1. **`deepseek-flash` over-verifies under `Ask`** (model behavior, not a rove
   defect — same as pilot-1 #7, amplified): the bounded-change run needed **8
   sequential tool approvals** because the model kept re-combining
   sha256sum/unittest/git-status instead of trusting earlier evidence. For
   long supervised sessions, `auto` is the practical policy; the `Ask` path
   still works exactly as designed.
2. **Trust-invalidation trap exercised twice for real**: every new session's
   `model-config` write changes the provider selector digest, which
   invalidates `provider_credentials` on the workspace — the queued
   follow-up then exhausts its 4 start attempts (`project_trust_required`)
   and lands the session in `needs_attention`. Recovery = re-grant trust +
   `POST /controls/{id}/confirm`. This is the designed path and it worked,
   but it is the single biggest friction point in the product flow and
   justifies the earlier decision to keep manual Recover.
3. **Fork at a mid-history run refused**: `fork_at_run_id` on run 6 (a
   `done` run, not the latest) returned `product_fork_source_invalid`
   "runtime job identity does not match its product binding"; forking at the
   newest run succeeded. Possibly correct strictness, possibly a defect —
   needs a repro test to decide. Recorded as a follow-up.
4. **Transcript projection cap**: `GET /transcript` returns `partial` with
   a typed `response_limit_reached` reason (run 7 tail truncated at seq
   362 of 2,941). Terminal state still projects from job state, so the UI is
   correct; the truncation is explicit, not silent.
5. **Model misjudgment under long history**: asked to recall the first
   turn, the model probed the durable-memory tool (12 lookups) instead of
   answering from context — the context itself was verified present in
   `task_state.json` (221 history entries, first user message verbatim).
   Not a runtime defect; noted for model-selection guidance.

## Secret hygiene

`grep -r "sk-4c5600"` over `ROVE_DATA_ROOT`, `ROVE_CONFIG_ROOT`, and all
evidence directories → **0 hits**. Provider key entered once via the
loopback onboarding endpoint into the OS keyring; never persisted, logged,
or echoed.

## Cost

~12 runs, each 500–18,000 events; estimated provider usage well under 1M
tokens (heaviest single run ~119k tokens reported by the UI).
