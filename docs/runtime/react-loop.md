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

The planned coordinator, `run_planned_loop` in `src/core/plan_loop.rs`,
delegates each current plan step to the bounded runner in
`src/core/step_runner.rs`:

1. Draft or resume a `TaskPlan`.
2. Convert the current plan step into a focused user prompt.
3. Build context with prior global history plus a step-local message prefix.
4. Run a model turn through the shared `run_model_turn` helper.
5. Execute a tool call or batch through the existing safety, approval, input,
   hook, and `run_tool_turn` path.
6. Append the assistant tool call and tool result to step-local history, then
   return that result to the model within the same plan step.
7. Keep recoverable tool errors in the same bounded step so the model can
   correct arguments or choose a safe alternative. Approval rejection remains
   fail closed and does not enter the replacement-plan path.
8. Mark the step complete only after `Action::Final` supplies the step
   conclusion. A successful tool call does not complete the step by itself.
9. Re-plan after a terminal recoverable step failure, using the existing
   replacement-`TaskPlan` compatibility behavior.

Step-local messages are included as prompt prefix material rather than ordinary
trimmable global history. This guarantees that the next model turn receives the
current tool result even when the configured global history window is zero or
older history is being compacted. When compaction changes the summary, the
runner rebuilds the actual model prompt before issuing the turn.

The engine now resolves the legacy `plan_enabled` and `max_steps` fields through
the typed `core::execution::ExecutionPolicy` boundary before selecting a loop.
`react` maps the old limit to `max_model_turns`, while `plan_react` maps it only
to `max_step_attempts`. Planned execution separately resolves
`max_model_turns_per_step = 4` as a named compatibility default; it does not
reinterpret `max_steps` as a second budget unit. Exhausting this step-local
ceiling emits the current `PlanStepFailed` event and completes the run with
`TerminationReason::StepLimit` plus an explicit
`max_model_turns_per_step=4` reason.

The public multidimensional budget configuration, structured budget-exhaustion
events, append-only `StepRecord` ledger, immutable plan revision chain,
Evaluator/Replanner policy, independent Finalizer, and their persistence/report
projections remain future work. Replacement plans still use `PlanCreated`, and
the current task state still stores `TaskPlan.steps[*].done` rather than the
future ledger.

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

For a planned step, that unit repeats until a step conclusion or a terminal
step outcome:

```text
StepRunner =
  bounded ReactTurn*
  -> Action::Final as step conclusion
  -> PlanStepCompleted
```

`Engine` is the orchestration shell. It loads resume state and memory, chooses
planned or unplanned mode, streams events, and writes run artifacts.
