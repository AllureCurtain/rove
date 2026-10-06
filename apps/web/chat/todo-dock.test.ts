import { describe, expect, it } from "vitest";

import type { TaskPlan } from "../lib/rove-types";
import { todoDockSummary } from "./todo-dock";

function plan(currentStep: number, done: boolean[]): TaskPlan {
  return {
    goal: "Ship the feature",
    current_step: currentStep,
    steps: done.map((value, index) => ({
      id: `step-${index + 1}`,
      title: `Step ${index + 1}`,
      done: value,
    })),
  };
}

describe("todoDockSummary", () => {
  it("counts done steps and names the indexed unfinished step", () => {
    const summary = todoDockSummary(plan(1, [true, false, false]));
    expect(summary).toMatchObject({ done: 1, total: 3 });
    expect(summary.current?.id).toBe("step-2");
  });

  it("falls back to the first unfinished step when the index lands on a done one", () => {
    // A skipped step keeps its slot, so current_step can sit on a finished row.
    const summary = todoDockSummary(plan(1, [false, true, false]));
    expect(summary.current?.id).toBe("step-1");
  });

  it("reports no current step once every step is done", () => {
    const summary = todoDockSummary(plan(3, [true, true, true]));
    expect(summary.done).toBe(3);
    expect(summary.current).toBeNull();
  });

  it("treats an out-of-range index as no indexed step", () => {
    const summary = todoDockSummary(plan(7, [false, false]));
    expect(summary.current?.id).toBe("step-1");
  });
});
