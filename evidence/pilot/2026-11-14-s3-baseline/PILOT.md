# Pilot S3 — real-model baseline out-of-the-box check

- **Date:** 2026-11-14
- **Operator:** ego-lite (browser-driven, no Playwright)
- **Build:** rove-api `target/debug` @ `5190b6b` ("real-model baseline — 128k context defaults + operator execution budgets"), `apps/web/web-dist` fresh
- **Provider:** DeepSeek `deepseek-v4-pro` via `https://api.deepseek.com` (OpenAI-compatible)
- **Env overrides:** NONE — no `ROVE_CONTEXT_*`, `ROVE_EXECUTION_*`, `ROVE_MAX_STEPS` set (verified `env | grep` empty before launch)
- **Roots:** `ROVE_CONFIG_ROOT=/tmp/rove-pilot3-config`, `ROVE_DATA_ROOT=/tmp/rove-pilot3-data`, port `127.0.0.1:18789`
- **Effective config:** `config.toml` contains only provider profile + `[model] default_model="deepseek-v4-pro"`; zero execution/context overrides. `context_hard_limit_tokens=128_000`, `context_soft_limit_tokens=96_000`, `context_reserved_tokens=16_000` (defaults, `apps/bootstrap/src/config.rs:493-495`). `max_model_turns_per_step=16` (derived `max_steps=20` policy projection, `selection_source=max_steps_and_plan_flag`).

## Pre-registered expectation

- Both turns complete `reason: final` / outcome success.
- No `context_token_limit`/`budget_exhausted`/`token_limit` termination on either turn.
- No operator budget rescue (only trust/first-grant + normal sends).
- PASS = both turns final + correct citations + zero budget death + zero manual budget rescue.

## Prompts

1. `In this repo, find where the agent kernel's tool-call loop dispatches a tool call to its handler. Tell me the file and the key function.`
2. `Now show me — by reading the code — what happens when a tool call needs user approval before it runs.`

## Actual

| | Turn 1 (run `01M7C72X74V5QFDW4565CVXX12`) | Turn 2 (run `01M7C77DNA1RJS3C3F59BMZ8VF`) |
|---|---|---|
| status | success | **incomplete** |
| termination_reason | **final** | **step_limit** |
| final_outcome | success | **exhausted** |
| model_turns | 17 (incl. planner) | 17 |
| step 1 model_turns_used | — | **16 / cap 16** → `model_turns_per_step_budget_exhausted` |
| tool_calls | 36 | 48 |
| total_tokens | 694,758 | 527,257 |
| plan | 2/2 steps complete | 2 steps planned, step 1 `budget_exhausted`, step 2 never ran |
| wall_time_ms | 102,657 | 11,141 (post-execution finalization segment) |

- Turn 1 Final answer: `crates/core/src/kernel.rs`, `run_agent_kernel` (line 203), dispatch `host.tool_turn(tool_action, cancel_token.clone())` at line 435; second hop `ToolRegistry::execute` `crates/core/src/tools.rs:314`; engine wrapper `rove-tools::Executor` `crates/tools/src/executor.rs`.
- Turn 2: UI banner `已达到执行限额，仍有工作未完成` / "step model-turn budget exhausted (max_model_turns_per_step=16)". No final answer emitted for the approval question.

## Termination reasons (trace)

- run1: `run_completed reason=final` — 0 budget/token-limit events.
- run2: `step_result status=budget_exhausted` (step 1, `error_code=model_turns_per_step_budget_exhausted`), `plan_decision finish_reason=budget_exhausted`, `finalization_completed outcome=exhausted`, `run_completed reason=step_limit`. 0 `context_token_limit`/`token_limit` events.

## Citations verified (independent grep)

- `crates/core/src/kernel.rs:203` `pub fn run_agent_kernel` — correct; `:435` is exactly `host.tool_turn(tool_action, cancel_token.clone())` — correct.
- `crates/core/src/tools.rs:300` `pub async fn execute`; line 314 `entry.tool.execute(args, ctx).await` — correct.
- `crates/tools/src/executor.rs:83` `self.registry.execute(name, args.clone(), ctx)` — correct.
- Turn-1 citations all accurate.

## Interventions

- UI "Verify and save" provider, workspace `provider_credentials` trust grant (first-run onboarding), two sends. **Zero budget rescues, zero env overrides, zero mid-run config edits.** Trust grant is expected first-run UX, not a rescue.

## Friction noted

- A transcript-restore defect surfaced at top of log: `run 2 (01M7C77DNA1R): the transcript response reached its safety limit. Expected event 2001, observed 9273.` with 重试恢复/新会话 buttons — history partially unrecoverable, new sends unaffected.
- Session `last_outcome` in product store = `failed`.

## Verdict

**FAIL** — Turn 2 terminated `step_limit`/`exhausted` on `model_turns_per_step_budget_exhausted` (cap 16). The new 128k context defaults held (context never the killer — 62.7k used after turn 1, ~527k total tokens run 2), but the baseline's own operator execution budget (`max_model_turns_per_step=16`) still kills a genuinely multi-step real-model task before it can answer, with no final answer produced. The model needed >16 model turns on plan step 1 alone.
