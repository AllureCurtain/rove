# Notes — S3 baseline retry

## What went right (the thing under test)
- Stock defaults: **no env overrides set**, fresh config+data roots.
- `max_model_turns_per_step: 48` propagated end-to-end: visible in `execution_strategy_selected.budgets` and every `execution_budget_updated` limit block in trace.jsonl.
- 19 model turns consumed before death — comfortably above the old 16 cap that killed last run's turn 2, so the headroom change is real and live.

## Why it still can't PASS
- DeepSeek account balance is **0**. Key authenticates (GET /models 200, lists `deepseek-flash`, `deepseek-v4-pro`), but every `chat/completions` call → `402 insufficient_balance_error` (verified directly with curl on both flash and chat).
- The runtime surfaced it cleanly: step 1 failed → `plan_decision replace_remaining` → deterministic finalizer → `run_completed reason: error outcome: partial`. No deadlock, no needs_attention, no stranded queue.

## Setup gotchas hit (for next run)
- Settings → 模型服务 is the provider form; "API 密钥" field is uncontrolled — fill works, but the advanced "密钥环境变量名" field (`OPENAI_API_KEY` placeholder) rejected `page.fill` because a `<fieldset>` intercepted pointer events. Worked around via JS `value` setter + `input` event. Set it to `DEEPSEEK_API_KEY`.
- First "验证并保存" click showed "密钥验证失败" but the profile had actually saved (saved-configs list + default-session dropdown updated). Second click confirmed "密钥已验证并安全保存". The verify toast appears flaky vs. the keyring save.
- Workspace onboarding: landing-page "手动输入路径" disclosure → absolute path → 类型 仓库 → opens session. Trust prompt appears inline ("授权" button) — one click grants `provider_credentials`.
- `page.waitForEvent` only supports popup/download (not "response"); use `page.cdp("Network.enable")` if wire capture needed.
- Session transcript API: `GET /product/sessions/{id}/transcript` → `{status, segments[].run_status, segments[].events[]}`. Run state files under `{ROVE_DATA_ROOT}/workspaces/{ws_id}/runs/{run_id}/`.

## Counts worth remembering
- Run id `01M7C84WV06KNH4XVCHTT6HNZS`: 5039 trace events, 16 plan steps executed→recorded as steps=16? report shows steps:16, tool_calls 32, 1 tool failure (`workspace file was not found` — model guessed a path), 651.2k total tokens, 57.7k context used.
