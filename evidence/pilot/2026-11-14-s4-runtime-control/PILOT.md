# Pilot S4 — runtime-control slice vs real provider

- Date: 2026-11-14
- Operator: ego-lite (ego-browser), driving the web console
- Env: rove @ `6693bbb` (`target/debug/rove-api`, `apps/web/web-dist` fresh); baseline `max_model_turns_per_step`=48, 128k context
- Provider: **deepseek-flash** via `https://api.deepseek.com` (OpenAI-compatible profile "DeepSeek")
- Isolated roots: `ROVE_CONFIG_ROOT=/tmp/rove-pilot4-config`, `ROVE_DATA_ROOT=/tmp/rove-pilot4-data`, port `18793`
- No `ROVE_*` budget overrides; no rebuild/modify/commit.

## Pre-registered expectations
- Cancel → terminal stopped, partial kept, no duplicate completed change, new task starts.
- Refresh → pending/run state restored, no silent resend/duplicate.
- Isolation → per-session model/trust/transcript correct; fork child independent.
- Overall PASS = all critical (cancel, refresh, isolation/fork) pass.

## Scenario results

| # | Scenario | Expected | Actual | Verdict |
|---|----------|----------|--------|---------|
| 1 | Setup | provider saved+verified, ws1 opened, trust granted | DeepSeek/`deepseek-flash` "密钥已验证并安全保存"; ws `rove` opened; "已授权 rove 使用提供商凭据" | PASS |
| 2 | Cancel mid-run | terminal cancelled, partial kept, clean follow-up | Clicked 停止 while `list_directory`/`search_code` running. UI: "执行已停止…记录中仍有 1 个步骤未完成", summary `outcome: cancelled`. `report.json` `status=cancelled, termination_reason=cancelled`. Follow-up "What was the first file you read?" started a NEW run → `success/final` with full Result. | PASS |
| 3 | Refresh recovery | run/pending restored, no resend/duplicate | `page.reload()` mid-run: session restored to 运行中, transcript streaming continued, plan dock + Stop restored. Later a `run_shell` approval surfaced and **persisted across the reload** (screenshot 04). `taskPromptCount` for the in-flight task = 1 → no duplicate send. | PASS |
| 4 | Session isolation + fork | per-session model/trust/transcript; fork independent | ws2 `/tmp/rove-pilot4-ws2` needed its OWN `provider_credentials` grant (did not inherit ws1's). Session 2 ran own task → `success`. Forked session 2 via 创建分支 → child `01M7CD8RA…` with `parent_session_id=…S2QR8`, `fork_point_run_id=01M7CCZXEH…`, `fork_point_seq=378`, inherited parent transcript, then continued independently (own `run_shell` approval → own answer). Parent untouched. | PASS |
| 5 | (bonus) Long-history | stable position, history loads, return-to-latest | In session 1 scrolled to top → gap banner "部分较早的消息未能恢复" (transcript safety-limit windowing), "加载更早的历史" paged back run2→run1; scroll-to-bottom returned to latest. | PASS |

## Overall: **PASS** (all critical scenarios pass)

## Notes / caveats
- **Fork-after-cancel is gated**: with session 1's latest run cancelled, the 创建分支 menu item was disabled and a direct API fork returned `product_fork_source_invalid` ("runtime job identity does not match its product binding"). `active_run_id` is `null` when idle and the UI fork requires `status==="idle" && activeRunId` + a cleanly-final latest turn. This is a real product boundary — a session whose last run ended `cancelled` cannot be forked from the UI (fork needs an *already-final completed* run). Forking session 2 (clean `done`) worked immediately.
- Transcript "response reached its safety limit" (expected seq 2001 vs observed 16351 on the cancelled run3) is transcript windowing, not lost state — earlier turns still paged in via "加载更早的历史".
- deepseek-flash is very thorough: the crates-enumeration task chained many `run_shell` approvals (each a one-shot grant); cancelled it to unblock the fork test.
- No `runaway`/duplicate side effects: all runs were read/`run_shell`(grep/ls) only; "duplicate" trace hits are the literal word inside grep payloads, not duplicated writes.

## Run ledger (data root `/tmp/rove-pilot4-data`)
| run_id | session | status | termination_reason | purpose |
|--------|---------|--------|--------------------|---------|
| 01M7CCMVTXCQXBFBFBMN7F0EV7 | s1 | cancelled | cancelled | task1 — Stop mid-run |
| 01M7CCNQF5ZN2HVER9MNFV0PKJ | s1 | success | final | follow-up after cancel |
| 01M7CCPPGRDV0KVC71X7N1PP5X | s1 | cancelled | cancelled | refresh task — survived reload, later stopped |
| 01M7CCZXEH4CBPS2YY64876WM5 | s2 | success | final | ws2 task; became fork_point_run_id |
| 01M7CD8Z0B6N2Y6Z0VWZZYWRHH | fork | success | final | fork's independent continuation |

## Session ids
- ws1 `rove` (`/Users/yao/dev/projects/rove`): session `01M7CCMAB8M8J1S2P3Q05M4FZQ`
- ws2 `rove-pilot4-ws2` (`/private/tmp/rove-pilot4-ws2`): session `01M7CCZMQJBX5VAMJ31EZS2QR8`
- fork (child of s2): `01M7CD8RA2N3B592MTKH9AJ591`

## Evidence files
- `01-mid-run-stop.png` — streaming tool calls + 停止 button
- `02-after-cancel.png` — "执行已停止" + cancelled summary
- `03-refresh-restored.png` — post-reload running state
- `04-refresh-pending-approval.png` — run_shell approval persisted across reload
- `05-second-workspace.png` — ws2 + own trust/session
- `06-fork-created.png` — child session nested under parent (派生 · event 378)
- `07-history-gap.png` — transcript gap banner + load-earlier
- `08-long-history.png` — paged-back history
- `session-export-*.json` — 3 session exports (lineage, run_models, transcript)
- `notes.md` — running notes
