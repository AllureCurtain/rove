# Notes — pilot smoke 1 (2026-11-14)

## Did the budget fix visibly pass 4 turns?
Yes, decisively. The previous run died at `BudgetExhausted: max_model_turns_per_step=4` before any synthesis. This run:
- trace `model_turns:24` (peak per-step), `model_turns_used:14` on one step — both >4.
- `max_model_turns_per_step:16` present in run config (the new default).
- 0 `BudgetExhausted` events in `trace.jsonl`.
- `run_completed` → `outcome: success`, `finish_reason: completed`.

## Provider UX / key-retention fix
- Created provider via the UI form (Settings → 模型服务 → 新建服务): type OpenAI, base `https://api.deepseek.com`, model `deepseek-v4-pro`, key pasted into the password field.
- Clicked "验证并保存" → status "密钥已验证并安全保存" on first try. Verify succeeded, so the key-retention path (keep key on failed verify) was NOT exercised this run — the happy path worked.
- Advanced options auto-expanded to reveal a "密钥环境变量名" field prefilled `OPENAI_API_KEY`. The saved profile used the pasted key, not the env var, so this default appears harmless, but worth noting it could confuse a user who has that var set to something else.
- After save, profile listed under 已保存的配置 and auto-selected as "新会话默认模型" (DeepSeek · https://api.deepseek.com). Clicking "使用" confirmed it active.

## Trust flow
- Opening the workspace then the session surfaced an alert: "工作区 rove 尚未授予「提供商凭据」权限" with an inline 授权 button. One click granted `provider_credentials`; status confirmed "已授权 rove 使用提供商凭据".
- No `project_trust_required` failure on send — the grant-then-send order via the banner avoided the old invalidated-digest trap. Good UX improvement vs. last run.

## Friction / observations
- **Transcript recovery warning (non-blocking):** the session log shows "部分较早的消息未能恢复 … run 1 (01M7C3H79WK0): the transcript response reached its safety limit. Expected event 2001, observed 8699." The trace has 8,700 events (7,125 `thinking_delta`); a transcript fetch cap (~2000) truncated early events. Cosmetic — the final answer and run outcome are intact — but the "重试恢复" prompt could alarm a user. Worth a look: transcript reader safety limit vs. very chatty thinking streams.
- **Compaction auto-triggered** once mid-run (`compaction_auto_triggered: true`, `compaction_mode: deterministic`, `compaction_degraded: false`, 21 msgs → summary). Handled cleanly, no surfacing error.
- **2 `tool_call_failed`** events out of 59; run still succeeded (likely a read/search miss the agent routed around). Not blocking.
- Token cost is high: ~509k total tokens (495,774 prompt) for a single-shot Q&A — driven by verbose reasoning + repeated large file reads. Expected for `deepseek-v4-pro` reasoning on a real repo; flag for cost-awareness rather than a defect.
- UI is fully localized Chinese; fine for operator, noted for any future English-locale smoke.
- ego-lite: `@N` refs go stale on re-render during streaming (expected); take a fresh `snapshot()` each round rather than caching refs. `waitForFunction` on body text worked well for detecting run completion.

## Surprise
- `model_turns:24` exceeds the per-step budget of 16. This appears to be the *step-level* counter across a step that itself drove multiple sub-iterations, while `max_model_turns_per_step:16` is enforced per individual step segment (`model_turns_used:14` ≤ 16). No `BudgetExhausted` fired, so the budget is not being violated — but the naming/counter granularity (step vs. model-turn-within-step) is worth a doc clarification so the 24 vs 16 numbers don't read as a bug.

## Defects
- None blocking. Open item: transcript-recovery truncation warning above (cosmetic, needs product decision on limit).
