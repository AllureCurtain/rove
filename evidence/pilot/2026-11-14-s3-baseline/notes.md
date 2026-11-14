# S3 notes — what the new defaults did and did not fix

## Held

- **Context budget is no longer the blocker.** With `context_hard=128k / soft=96k / reserved=16k` defaults, neither turn hit `context_token_limit`/`token_limit`. Context used after turn 1 was 62,687 (UI meter) — comfortably under soft limit; no compaction pressure observed.
- **Session continuity worked.** Turn 2 ran in the same session, streamed thinking + tool calls, incremented the token meter normally. The 128k default let a 695k-prompt-token turn 1 and a 527k run 2 complete model calls without context death.
- **Out-of-the-box onboarding is clean.** Provider verify/save, workspace trust grant, send — no console edits, no env, no `model-config` PUT needed by hand (the web flow did it). Zero budget rescues required to *start* either turn.

## Did NOT hold — the actual FAIL

- **`max_model_turns_per_step=16` is still too small for a real multi-step task.** Turn 2 (a read-and-explain question) planned 2 steps; step 1 alone consumed all 16 model turns and 48 tool calls exploring the approval path, then died `model_turns_per_step_budget_exhausted` → run `step_limit`, `outcome=exhausted`, step 2 never ran, no final answer. The budget that killed S2's class of task is still the default.
- This is the *baseline's own derived default* (`max_steps=20` → `selection_source=max_steps_and_plan_flag`), not an env var — so "no overrides" faithfully reproduces the S2 failure mode on the execution-budget axis even though the context axis is fixed.

## Friction / defects worth filing

1. **Transcript recovery safety limit** — top-of-log banner: `the transcript response reached its safety limit. Expected event 2001, observed 9273` for run 2; earlier messages unrecoverable (重试恢复/新会话 buttons shown). New sends unaffected but history paging broke on a 9k-event run.
2. **`last_outcome=failed` on session** — consistent with exhausted run, but means the session list will badge this session as failed.
3. DeepSeek spent heavily (527k prompt tokens) looping reads on step 1; per-step turn cap interacts badly with a verbose exploratory model.

## Verdict

FAIL. New 128k context defaults work, but the real-model baseline still cannot finish a two-turn read/explain task out of the box: the second turn dies on `max_model_turns_per_step=16`. S3's bar — "no budget rescue needed" — is not met.
