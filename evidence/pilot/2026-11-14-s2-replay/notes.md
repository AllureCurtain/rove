# S2 pilot — defects & friction notes

Verdict: PASS on correctness, but it took real operator work to get there. These are the friction/defect findings.

## 1. Default context token budget is far too small for a real coding model
- `context_hard_limit_tokens` defaults to **30,000** (`apps/bootstrap/src/config.rs`); soft 24k, reserved 4k.
- DeepSeek `deepseek-v4-pro` reads whole files; a single search+read turn can produce tens of thousands of prompt tokens. Two consecutive runs died with `termination_reason: token_limit` / `error_code: context_token_limit` ("context exceeds configured hard token budget") before doing any useful work.
- Workaround used: relaunch with `ROVE_CONTEXT_SOFT_LIMIT_TOKENS=160000`, `ROVE_CONTEXT_HARD_LIMIT_TOKENS=200000`, `ROVE_CONTEXT_RESERVED_TOKENS=8000`.
- Friction: there is NO env var or user-config field for `max_model_turns_per_step` or `runtime.execution.*` — the user config file only accepts `schema_version`/`model`/`provider` (a stray `[runtime]` block fails startup with `unknown field 'runtime'`). Execution budgets can only be set via project `.rove/config.toml` (which would pollute the agent's diff) or code.

## 2. Per-step model-turn budget (16) is too tight for an exploratory agent
- `DEFAULT_MAX_MODEL_TURNS_PER_STEP = 16` (`crates/foundation/src/execution.rs`). Even after the token fix, the run died on `step_limit`/`budget_exhausted` ("exhausting its ModelTurnsPerStep budget") after 58 tool calls — all inside plan-step 1.
- DeepSeek over-explored: 55 `search_code`/`read_file` calls, never advancing to the edit step. The agent had ALREADY diagnosed the cap correctly (its reasoning cited `MAX_EVENTS_PER_RUN=2_000` and even spotted that `docs/api.md` already says 20,000) but kept gathering instead of editing.
- Workaround: a directive follow-up naming the exact file/constants/values forced it straight to `edit_file`. With that nudge the agent completed cleanly in ~4 min / 7 tool calls / 74k tokens.
- Takeaway: for this model the bottleneck was turn-count-per-step, not tokens. A model that defaults to "act sooner" would not need the nudge.

## 3. Stranded-queue / needs_attention deadlock recurs on every failed turn
- Each failed run leaves the session in `status: needs_attention` with "session requires an explicit recovery decision". The queued follow-up cannot start until the operator clicks "恢复会话" (Recover session).
- This is the documented stranded-queue trap (see rove-deepseek-testing memory). It happened on EVERY retry (4 controls created; each new message needed an explicit Recover click). UI lag made the recovery row appear/disappear; the underlying control was always `applied` to the failed run.

## 4. Product UX observations (non-blocking)
- The console is fully Chinese-localised; aria-labels are stable so automation still worked.
- `max_steps` (default 8) is a separate budget from context tokens AND from per-step turns — three distinct limits can each independently kill a run, which is confusing to reason about (`token_limit`, `step_limit`, `budget_exhausted`).

## 5. Meta-irony (interesting, not a defect)
- The session's OWN exported transcript recorded `partial_reasons: [{"code":"response_limit_reached","expected_seq":2001,"observed_seq":2638,"run_id":"...T7"}]` — the very truncation bug under test fired on the failing run's own transcript. A nice real-world reproduction of the symptom the agent was asked to fix.

## Operator interventions (for the record)
1. `PUT model-config` `max_steps` 8→40.
2. Relaunched rove-api with the three `ROVE_CONTEXT_*` env overrides.
3. Clicked "恢复会话" several times to drain stranded queue.
4. Sent one directive message naming file/constants/values (broke the exploration loop).
5. Approved the `edit_file` write in the approval UI.
No intervention touched the diagnosis or the diff content beyond the directive nudge.
