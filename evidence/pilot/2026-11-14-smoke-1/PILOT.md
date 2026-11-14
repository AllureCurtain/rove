# Pilot Smoke 1 — budget-fix re-run (also pilot task S1: code-understanding)

- **Date**: 2026-11-14
- **Operator**: ego-lite (ego-browser), unattended
- **Env**: macOS (Darwin 27.2.0); rove `main` @ `d8aec71`; binary `target/debug/rove-api` (built 12:01); `apps/web/web-dist` fresh.
- **Server**: `rove-api --addr 127.0.0.1:18788 --web-dist apps/web/web-dist`, fresh `ROVE_CONFIG_ROOT=/tmp/rove-pilot1-config`, `ROVE_DATA_ROOT=/tmp/rove-pilot1-data`, `TMPDIR=/private/tmp`.
- **Provider**: DeepSeek, OpenAI-compatible profile, base `https://api.deepseek.com`, model `deepseek-v4-pro`. Key from env (not stored in any file/screenshot).
- **Budget under test**: `max_model_turns_per_step` default raised 4→16 (commit `717b156`). Prior run died at `BudgetExhausted: max_model_turns_per_step=4` before synthesis.

## Prompt
> Where is the agent's main run loop implemented? Cite file paths and line numbers.

## Expected
Real streamed run that exceeds 4 model turns on a step, uses search/read tools, and reaches a Final answer with plausible file:line citations — no `BudgetExhausted`.

## Actual
Run completed end-to-end with `outcome: success`, `finish_reason: completed`, `reason: final`.

- **Runtime**: run `01M7C3H79WK0ES8DJD4P1HBYDY`, session `01M7C3GYSWDWVG23WVPTKD00WG`.
- **Steps**: 23 plan-driven units of work over 2 plan steps (plan dock "已完成 2/2 · 全部步骤已完成").
- **Tool calls**: 59 started / 57 completed / 2 failed (non-fatal).
- **Model turns**: trace shows `model_turns:24` peak on a step and `model_turns_used:14` — far past the old cap of 4. `max_model_turns_per_step:16` in run config. **0 `BudgetExhausted` events.**
- **Tokens**: 508,973 total (495,774 prompt / 13,199 completion). Context meter "已用 31,301"; compaction auto-triggered once (`compaction_auto_triggered: true`, 21 messages compacted), not degraded.
- **Elapsed**: ~2:03 wall.
- **Interventions**: none after send. One manual trust grant via the in-app "授权" banner (expected flow).

## Answer correctness (verified against repo)
Every citation checked with `grep`/`sed` — all exact:

| Cited | Verified |
| --- | --- |
| `run_unplanned_loop` @ `crates/engine/src/engine/run_loop.rs:427` | ✓ line 427 |
| loop body `while let Some(item)=kernel.next().await` @ :490 | ✓ line 490 |
| `run_kernel_model_turn` @ `run_loop.rs:1024` | ✓ line 1024 |
| `run_planned_loop` @ `plan_loop.rs:45` | ✓ line 45 |
| plan loop `while let Some(active_plan)` @ :281 | ✓ line 281 |
| `run_agent_kernel` @ `crates/core/src/kernel.rs:203` | ✓ line 203 |
| `Engine::run` @ `facade.rs:656`, `run_with_cancel` @ :661 | ✓ 656/661 |
| dispatch `run_planned_loop`/`run_unplanned_loop` @ :1221/:1241 | ✓ 1221/1241 |
| `run_agent_kernel` call site @ `run_loop.rs:479` | ✓ line 479 |

## Verdict: **GO**

Provider configured through the UI form (verify+save kept the key and reported "密钥已验证并安全保存"), a real streamed run exceeded 4 model turns per step by a wide margin (peak 24), reached a Final answer whose file:line citations are all exactly correct, no crash, evidence saved.

## Files
- `01-provider.png` — provider verified & saved, set as new-session default.
- `02-streaming.png` — mid-run: multiple thinking/tool-call blocks (>4 model turns), plan dock 0/2.
- `03-final-answer.png` — final cited answer + "本轮执行已完成", plan 2/2.
- `04-usage-meter.png` — same frame; context meter "已用 31,301".
- `session-export.json`, `transcript.json`, `usage.json`, `trace.jsonl`, `task_state.json`.
