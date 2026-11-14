# Pilot smoke test — rove web console + real DeepSeek provider

| Field | Value |
| --- | --- |
| Date | 2026-11-14 |
| Operator | ego-lite browser, driven by a subagent (Claude Code) |
| Environment | macOS (Darwin 27.2.0), rove `main` @ `1891894`, `rove-api` on `127.0.0.1:18787`, `web-dist` static console |
| Provider | OpenAI-compatible profile → `https://api.deepseek.com/v1` |
| Model | `deepseek-v4-pro` (see defect D1 — `deepseek-chat` no longer exists) |
| Key source | user-supplied DeepSeek key (via credential field; stored to OS keyring, never written to any file) |
| Config roots | `ROVE_CONFIG_ROOT=/tmp/rove-pilot-config`, `ROVE_DATA_ROOT=/tmp/rove-pilot-data`, `TMPDIR=/private/tmp` |
| Task prompt | `Where in the codebase is the agent's main run loop implemented? Cite file paths and line numbers.` |
| Workspace | `/Users/yao/dev/projects/rove` (the rove repo itself) |

## Expected outcome

Provider configured through the UI, a real streamed run that calls tools against the
repo and returns a natural-language answer citing real `file:line` locations.

## Actual outcome

- Provider onboarding via the **UI form succeeded** (`Key verified and stored securely`),
  profile `DeepSeek` saved and set as default for new sessions.
- Workspace opened; `provider_credentials` trust granted via the Authorize prompt.
- **Three separate runs** of the task all streamed real model output and executed real
  tools (`search_code`, `read_file`, …) against the repo — 7–14 tool calls, 21–33k
  tokens per run. No crash, no auth error, no empty response.
- **Every run terminated with `BudgetExhausted: step model-turn budget exhausted
  (max_model_turns_per_step=4)` and produced NO final answer** — only the auto-generated
  "Execution reached its limit" summary. The agent spent all 4 model turns issuing
  parallel tool batches and never reached the synthesis turn that would write citations.

## Answer quality

N/A — no final natural-language answer was ever produced, so there were no citations to
verify. Tool trajectory was correct (it was searching the `engine` crate, which is where
the run loop actually lives: `crates/engine/src/engine/run_loop.rs`, `model_turn.rs`,
`plan_loop.rs`, `crates/core/src/kernel.rs`).

## Elapsed

~7 min wall-clock from API up to third exhausted run (includes provider setup +
3 runs + a restart).

## Token usage (from UI / export)

- Run 1: 7 actions · 21.4k tokens
- Run 2: 14 actions · 33.2k tokens
- Run 3: 13 actions · 22.6k tokens

## Intervention count

- 1 re-entry of the API key after the first "Verify and save" consumed it without saving (D2).
- 1 model change `deepseek-chat` → `deepseek-v4-pro` forced by live `/models` (D1).
- 1 grant of `project_configuration` + write of `.rove/config.toml` budget override (D3, ineffective).
- 2 API restarts attempting to make the budget override take effect (ineffective).

## Defects found — see notes.md

- **D1** `deepseek-chat` removed from DeepSeek's live `/models`; brief's model name is stale.
- **D2** "Verify and save" consumes the entered key but does not persist the profile when
  model validation fails → key silently lost, must re-enter. Chicken-and-egg: "List models"
  needs a saved/env credential, but you can't save without a valid listed model.
- **D3** `runtime.execution.max_model_turns_per_step` in project `.rove/config.toml` is
  **not honored** for API/product sessions even with `project_configuration` granted +
  restart — hardwired default of 4 cannot be raised by any operator-reachable means.
- **D4** Default `max_model_turns_per_step = 4` is too low for a tool-using research task;
  each thinking→tool-batch turn consumes a model turn, so synthesis never runs.

## Verdict

**NO-GO.**

The transport, provider onboarding, tool execution, streaming, and token accounting all
work end-to-end against the real DeepSeek provider — that half of the pilot is a clear
pass. But the primary success criterion (a real streamed answer with plausible file:line
citations) is unreachable: the hardwired `max_model_turns_per_step=4` budget kills every
research run before the answer-synthesis turn, and no UI/API/honored-config path lets an
operator raise it. **Top-priority dev task: expose/raise the per-step model-turn budget
(or fix project-config `runtime.execution` to apply to product sessions) so a
tool-using run can reach its final answer.**
