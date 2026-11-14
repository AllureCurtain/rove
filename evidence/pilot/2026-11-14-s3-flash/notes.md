# Notes — pilot S3 flash

## Setup sequence that worked (repeat of prior gotchas, all applied)

- Settings → 模型服务: name `DeepSeek Flash`, type OpenAI, base `https://api.deepseek.com`, default model `deepseek-flash`, key pasted into "API 密钥" (uncontrolled input — `page.fill` works).
- Advanced "密钥环境变量名" needed JS value setter + `input`/`change` dispatch to `DEEPSEEK_API_KEY` (fieldset intercepts pointer events; `page.fill` rejected).
- One "验证并保存" click → toast "密钥已验证并安全保存" (single attempt this time; no false-negative verify toast observed).
- Default-session mode flipped to "已保存配置 / DeepSeek Flash / deepseek-flash" automatically.
- Workspace: 手动输入路径 → `/Users/yao/dev/projects/rove` → 类型 `仓库` → 打开工作区. Session auto-created with `deepseek-flash` model chip already selected.
- Trust: inline "授权" button granted `provider_credentials` — single click, status "已授权 rove 使用提供商凭据".

## Runtime observations

- Turn durations: turn 1 ~4 min (05:58–06:02), turn 2 ~7 min (06:02–06:09). Long but steady streaming throughout; thinking deltas dominate the trace (19k / 25k thinking_delta events).
- `deepseek-flash` explores differently than the earlier `deepseek-v4-pro` run: fewer model turns (35/31 vs prior cap-16 death) but very high prompt-token volume (1.2M / 1.7M total) — it reads wide file ranges and enumerates systematically via `repository_map`/`glob`/`search_code`.
- Context meter read 64,521 then 94,354 used — "模型未报告窗口大小" (DeepSeek completions don't report context window; the 128k default applied silently).
- Turn 2's single `tool_call_failed` was `invalid_args: context must be ≤ maximum 10` — self-corrected on the next call.
- UI transcript notice "部分较早的消息未能恢复 / response_limit_reached (expected seq 2001, observed 23470)" after turn 1: export/transcript replay pagination cap; run unaffected. Reproduces the same export `partial_reasons` entry in `session-export.json`.
- Session API note: `GET /product/sessions/{id}` is 405 (only PATCH/DELETE); use `GET /product/sessions?workspace_id=…` list for status, `POST /product/sessions/{id}/export` for the export bundle.

## Evidence inventory

- `PILOT.md` — the report (PASS).
- `final-turn1.png`, `final-turn2.png` — screenshots at each turn's completion state.
- `session-export.json` — `/product/sessions/{id}/export` bundle (2 runs, model `deepseek-flash`, wire `openai-completions`).
- `trace-turn1.jsonl` / `trace-turn2.jsonl` — raw `trace.jsonl` from both runs.
- `report-turn1.json` / `report-turn2.json` — derived `report.json` (termination_reason final, outcome success, budgets).
