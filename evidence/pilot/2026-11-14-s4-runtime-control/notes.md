# S4 runtime-control — running notes

- Server: `rove-api` on 127.0.0.1:18793, fresh `/tmp/rove-pilot4-{config,data}`. Health OK at first probe.
- Provider: Settings → 模型服务 → New service "DeepSeek" (type OpenAI), base `https://api.deepseek.com`, default model `deepseek-flash`. Pasted key via JS value-setter into `input[placeholder="sk-…"]` (uncontrolled). Set 密钥环境变量名 → `DEEPSEEK_API_KEY`. "验证并保存" → toast "密钥已验证并安全保存"; profile auto-selected as default for new sessions.
- ws1 `/Users/yao/dev/projects/rove` (kind repo) opened via 手动输入路径. Auto-created "New session" with deepseek-flash. Trust banner → 授权 → granted `provider_credentials`.

## Cancel
- Sent the trace-plan prompt. Within ~12s it formed a 4-step plan and had list_directory + search_code running; send button became 停止. Clicked → transcript showed "执行已停止，部分工作可能尚未完成" + auto summary `outcome: cancelled`, "1 个步骤未完成". report.json confirmed `cancelled/cancelled`. Send button restored (idle). Follow-up "What was the first file you read?" launched a fresh run that completed with a structured Result naming `crates/protocol/src/lifecycle.rs`.

## Refresh
- Sent crates-enumeration task. Mid-run `page.reload()` → session restored 运行中, transcript resumed streaming, plan dock + Stop present. The run then surfaced a `run_shell` approval which survived the reload (screenshot 04). Confirmed only 1 copy of the user prompt in the log → no resend.
- The agent kept requesting `run_shell` approvals (grep/ls sweeps); approved several, then stopped the run to free the session for the fork test.

## Isolation + fork
- ws2 `/tmp/rove-pilot4-ws2` (3 shell scripts, kind folder). Its session needed a SEPARATE trust grant — ws1's did not carry over. Session 2 task → `success/final`.
- Tried fork on session 1 first: menu 创建分支 **disabled**, API `POST /forks` → `product_fork_source_invalid`. Cause: latest run was cancelled; `active_run_id` is null when idle and UI requires `idle && activeRunId` (a final completed turn). Gated correctly.
- Forked session 2 (clean done): child `01M7CD8RA…` nested under parent with `派生 · 01M7CCZXEH · event 378`. Lineage in export: `parent_session_id`, `fork_point_run_id`, `fork_point_seq=378`. Sent a different prompt → fork ran its own plan and `run_shell` approval, answered independently. Parent unchanged (still idle, parent_session_id null).

## Long-history
- Session 1, scrolled up: gap banner "部分较早的消息未能恢复" + per-run "response reached its safety limit" (event windowing cap). "加载更早的历史" paged back one run at a time (run3→run2→run1). Scroll-to-bottom returned to latest. Position stable.

## Gotchas for next time
- 创建分支 disabled = session's latest turn isn't a final completed run (e.g. just cancelled). Fork a cleanly-`done` session instead.
- `page.fill` on `input[placeholder="sk-…"]` didn't take — JS setter + input event worked.
- Session-row 操作 button is hover-revealed & zero-sized until hovered; a JS click on the aria-label match opens the menu.
- deepseek-flash on read-heavy enumeration loves chained run_shell approvals; keep tasks small or pre-grant.
