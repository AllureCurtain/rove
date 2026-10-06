import type { PlanStep, TaskPlan } from "../lib/rove-types";

/**
 * The composer todo fold's reading of a run plan (design §6): how many steps
 * are done, and which one the run is on. The plan's own `current_step` leads
 * while it still points at an unfinished step; once the runtime advances past
 * it (the reducer writes `steps.length` when nothing remains), the summary
 * falls back to the first unfinished step — and to null when all are done, so
 * the header can say so instead of naming a finished step "current".
 */
export function todoDockSummary(plan: TaskPlan): {
  done: number;
  total: number;
  current: PlanStep | null;
} {
  const steps = plan.steps;
  const done = steps.reduce((count, step) => (step.done ? count + 1 : count), 0);
  const indexed = steps[plan.current_step];
  const current =
    indexed && !indexed.done ? indexed : (steps.find((step) => !step.done) ?? null);
  return { done, total: steps.length, current };
}
