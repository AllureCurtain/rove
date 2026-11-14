# Pilot S3 — deepseek-flash real-provider multi-turn run

- Date: 2026-11-14
- Operator: ego-lite (ego-browser v2.2.1, TaskSpace 27)
- Build under test: rove @ main `6693bbb` (`target/debug/rove-api`, `apps/web/web-dist`), defaults only
- Env: `ROVE_CONFIG_ROOT=/tmp/rove-pilot3c-config`, `ROVE_DATA_ROOT=/tmp/rove-pilot3c-data`, `TMPDIR=/private/tmp`, port `127.0.0.1:18792`. **No** `ROVE_CONTEXT_*` / `ROVE_EXECUTION_*` / `ROVE_MAX_STEPS` overrides.
- Provider: DeepSeek, base `https://api.deepseek.com`, model **`deepseek-flash`**, wire `openai-completions`, key via `DEEPSEEK_API_KEY` env.
- Workspace: `/Users/yao/dev/projects/rove` (repo kind), `provider_credentials` trust granted via inline 授权.

## Prompts

1. Turn 1: "In this repo, find where the agent kernel's tool-call loop dispatches a tool call to its handler. Tell me the file and the key function."
2. Turn 2 (exploratory; died at cap 16 in the earlier run): "Now show me — by reading the code — what happens when a tool call needs user approval before it runs."

## Pre-registered expectation

Both turns `final`/success; turn 2 gets headroom up to 48 model turns; zero `BudgetExhausted`/`token_limit`/`step_limit`/`402`; zero manual rescue.

## Actual

| | Turn 1 | Turn 2 |
|---|---|---|
| run_id | `01M7CA6QXYWHR8SCYFBNMEM2TB` | `01M7CADNEGMCV90C08FVRBJR15` |
| status / termination_reason | success / **final** | success / **final** |
| plan steps | 3/3 succeeded | 3/3 succeeded |
| model_turns (budget_used) | **35** | **31** |
| tool_calls | 71 (all ok) | 69 (68 ok, 1 invalid_args → retried ok) |
| total tokens (report) | 1,256,946 (prompt 1,227,944 / completion 29,002) | 1,726,748 (prompt 1,688,430 / completion 38,318) |
| context meter at end | 64,521 | 94,354 |
| max_model_turns_per_step in effect | 48 | 48 |
| max_steps (plan) | 8 (3 used) | 8 (3 used) |

## Deaths / rescues

- `BudgetExhausted`: **0** in both traces.
- `context_token_limit` / `step_limit` / `insufficient_balance` / `402`: **0**.
- `needs_attention` deadlock: **none**.
- Manual interventions during runs: **0** (only normal UI setup: provider save+verify, workspace open, trust grant).
- Trace "error"/"ToolCallLimit" grep hits are false positives — they are the agent's own `search_code` results *for the string* `ToolCallLimit` inside `kernel.rs`, not runtime failures.
- One `tool_call_failed` in turn 2: `invalid_args` ("context must be ≤ 10") on a `search_code` call; agent corrected args and succeeded on retry. Not a runtime defect.
- Cosmetic only: transcript UI showed a "partial recovery" notice for run 1 (`response_limit_reached`, export seq gap expected 2001 observed 23470) — a trace-streaming pagination limit in the product export path, not a run failure; the run itself completed `final`/success.

## Independent citation check (repo grep)

- Turn 1: `crates/core/src/tools.rs:300` `pub async fn execute` ✓; `crates/core/src/kernel.rs:435` `host.tool_turn(tool_action, …)` ✓; `kernel.rs:203` `run_agent_kernel` ✓; `crates/core/src/agent.rs:441` `tools.execute(&invocation.name …)` ✓. Answer correctly distinguishes kernel dispatch point (kernel.rs:435 → host stream) from name→handler resolution (ToolRegistry::execute → `entry.tool.execute` at tools.rs:314).
- Turn 2: `crates/tools/src/executor.rs:77` `check_tool_allowed(&schema, services.approval_policy)?` ✓; `crates/foundation/src/boundary.rs:7` `pub fn check_tool_allowed` ✓ with exact reasons "destructive tool blocked by policy" (:13) and "destructive tool requires explicit approval" (:16) ✓; `crates/engine/src/engine/tool_turn.rs:281` `begin_approval` + pending-approval selects at :484/:569 ✓. Answer correctly reports approve/deny/no-response branches (fail-closed everywhere, no approval timeout).

## Verdict

**PASS** — both turns reached `termination_reason: final` / `outcome: success` out of the box on `deepseek-flash`, with correct, line-accurate citations, zero budget/provider deaths, and zero manual rescue. The raised `max_model_turns_per_step = 48` held comfortably (peak consumption 35 model turns, ~73% of cap).
