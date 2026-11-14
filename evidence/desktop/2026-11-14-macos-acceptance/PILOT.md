# Desktop acceptance — macOS arm64

Date: 2026-11-14. Build: `cargo tauri build --bundles app` on `main`
(`5f03999`, post PR #97+#98). Host: owner's Apple Silicon machine,
macOS 27.2. Binary: `~/Applications/Rove.app` (63 MiB, adhoc-signed,
`com.rove.agent`, `developer-tools`, copyright `AllureCurtain`).

Isolation: `ROVE_DESKTOP_ROOT=/tmp/rove-desktop-acceptance` so the real
`~/Library/Application Support/Rove` was untouched. Workspace under test:
`/tmp/rove-acceptance-workspace` (git repo). Provider: live DeepSeek
`deepseek-flash`, key entered through the loopback onboarding route into
the macOS keyring (never written to disk or evidence). The embedded API
was driven over `127.0.0.1` with the bearer token from
`state/config/desktop.json`; the WebView rendered throughout (WebKit
`com.rove.agent` data present).

## Result matrix

| Scenario | Result | Evidence |
| --- | --- | --- |
| Install → first launch → onboard provider | PASS | `.app` launched from `~/Applications`; embedded API on `127.0.0.1` + preview origin bound; onboarding probed live DeepSeek (`inventory_count 2`, streaming/native-tools/usage supported), credential landed in OS keyring (`credential_source: keyring`) |
| Workspace pick + trust + run a turn | PASS | Workspace registered (`kind repo`), all six trust capabilities granted incl. `provider_credentials`; real model planned, read `README.txt`, `run_completed {reason: final}` |
| Approve tool call | PASS | `plan_approval_needed` → `POST /jobs/{id}/plan-approvals/{id} {mode: ask}` → `plan_approval_resolved`, run resumed |
| Deny tool call | PASS | `write_file` pending → `POST .../approvals/{call_id} {decision: reject}` → file NOT created, run ended `error` |
| Close window mid-run → reopen → state intact | PASS | Two paths: hard SIGTERM and real `osascript quit`. Quit produced `API graceful shutdown initiated` → `post-run hook execution cancelled` → `embedded API server shut down gracefully`. After relaunch, session `error`/`last_outcome failed`, both run segments (130 + 903 events) and the `needs_attention` message all present |
| Second instance refused | PASS | Same `ROVE_DESKTOP_ROOT`, second process exited during ownership acquire; first held `.rove-api.lock` throughout |
| Uninstall → data dir preserved | PASS | `Rove.app` deleted; 4.3 MB state dir (product.sqlite, 5 trace.jsonl, config token) fully intact |
| Stranded-queue Recover (bonus) | PASS | Post-restart `needs_attention` message → `POST .../controls/{id}/confirm` → `claimed_successor` → real run started |

## Trust/trap notes

- Hit the documented trap: granting `provider_credentials` before the
  provider profile was projected (`model-config`) left the trust invalid;
  the turn requeued `project_trust_required` until re-granted after the
  profile was final. Order that works: onboard → workspace → trust →
  model-config → (re-grant if profile edited) → send.
- `GET /product/sessions/{id}` is not a route; session state read via
  `GET /product/sessions?workspace_id=` and `/transcript`.

## Open finding (needs repro, not a desktop-host defect)

Turn 3's `plan_approval_resolved.approved_policy` recorded `auto` although
the resolve request sent `mode: ask`, and a `resume runtime identity
mismatch ["approval_policy"]` warning appeared on the next turn. Two
candidate causes: (a) the plan-gate mode→policy mapping or the recorded
`approved_policy` field is wrong, or (b) a side effect of the resolve
being POSTed ~8× by a polling loop that re-read `plan_approval_needed`
from the transcript (only the first can resolve; later ones 404). Needs a
single-shot repro to classify defect vs. correct strictness — same class
of question as the `product_fork_source_invalid` follow-up already in
TODO. Filed as a follow-up, not blocking.

## Verdict

macOS `.app` install-to-run acceptance: **PASS** on all six matrix cells.
The remaining blockers for real distribution stay in TODO/Blocked:
code signing + notarization, clean-machine validation.
