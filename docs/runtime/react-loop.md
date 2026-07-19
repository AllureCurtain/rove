# Runtime Loop: Plan Outside, ReAct Inside

rove uses a Plan + ReAct runtime shape.

The unplanned loop in `src/core/run_loop.rs` is the pure ReAct loop implemented by
`run_unplanned_loop`:

1. Build context with `ContextManager::build_with_checkpoint`.
2. Compact old history when the token budget requires it.
3. Run one model turn through `run_model_turn`.
4. Normalize native OpenAI, Anthropic, Ollama, and Responses tool-use into `Action`.
5. Run one tool turn through `run_tool_turn`.
6. Append assistant tool calls and tool results back into history.
7. Repeat until final answer, cancellation, token limit, step limit, or error.

The planned loop in `src/core/plan_loop.rs` keeps the same ReAct core inside each
plan step through `run_planned_loop`:

1. Draft or resume a `TaskPlan`.
2. Convert the current plan step into a focused user prompt.
3. Run the same model turn and tool turn as the unplanned loop.
4. Mark the step complete on success.
5. Re-plan when a step fails and the failure is recoverable.

The engine now resolves the legacy `plan_enabled` and `max_steps` fields through
the typed `core::execution::ExecutionPolicy` boundary before selecting a loop.
This is a compatibility foundation only: `react` maps the old limit to
`max_model_turns`, while `plan_react` maps it to `max_step_attempts`. The
bounded StepRunner, append-only ledger, plan revisions, and evaluator remain
future work and are not implied by these types.

This differs from pico's `pico/agent_loop.py`, where prompt build, model call,
parse, tool execution, checkpoint, and trace recording live in one readable loop.
rove keeps those phases split into focused Rust modules so provider streaming,
cancellation, artifact persistence, API jobs, and plan recovery can evolve
independently.

The conceptual ReAct unit in rove is:

```text
ReactTurn =
  ContextBuild
  -> ModelTurn
  -> Action
  -> ToolTurn
  -> HistoryAppend
```

`Engine` is the orchestration shell. It loads resume state and memory, chooses
planned or unplanned mode, streams events, and writes run artifacts.
