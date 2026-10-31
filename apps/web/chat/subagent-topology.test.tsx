import { renderToStaticMarkup } from "react-dom/server";
import { createElement } from "react";
import { describe, expect, it } from "vitest";

import { CopyProvider } from "../copy/CopyProvider";
import type { SubagentView, ToolCallView } from "../lib/rove-state";
import { SubagentTopology, subagentRowLabel } from "./subagent-topology";

function parentTask(overrides: Partial<ToolCallView> = {}): ToolCallView {
  return {
    id: "call-task-1",
    name: "task",
    status: "running",
    details: "",
    args: { prompt: "Survey the repo" },
    ...overrides,
  };
}

function child(
  parentCallId: string,
  overrides: Partial<SubagentView> = {},
): SubagentView {
  return {
    parentCallId,
    status: "running",
    eventCount: 3,
    messages: [],
    tools: [],
    ...overrides,
  };
}

function render(element: React.ReactElement) {
  return renderToStaticMarkup(
    createElement(CopyProvider, null, element),
  );
}

describe("SubagentTopology", () => {
  it("draws the coordinator line and one row per delegated child", () => {
    const html = render(
      <SubagentTopology
        tool={parentTask()}
        subagents={[child("call-task-1")]}
      />,
    );
    expect(html).toContain("subagent-topology");
    expect(html).toContain("正在编排 1 个代理");
    expect(html).toContain('role="list"');
    // The row names the delegated prompt's first line when no agent arg exists.
    expect(html).toContain("Survey the repo");
    expect(html).toContain("3 步");
  });

  it("uses the agent arg as the row name when the parent requested one", () => {
    const html = render(
      <SubagentTopology
        tool={parentTask({ args: { prompt: "p", agent: "explorer" } })}
        subagents={[child("call-task-1")]}
      />,
    );
    expect(html).toContain("explorer");
    expect(html).not.toContain(">p<");
  });

  it("offers stop only on a running row and only with a handler", () => {
    const running = render(
      <SubagentTopology
        tool={parentTask()}
        subagents={[child("call-task-1")]}
        onStopSubagent={() => undefined}
      />,
    );
    expect(running).toContain("subagent-stop");
    expect(running).toContain(">停止<");

    const settled = render(
      <SubagentTopology
        tool={parentTask({ status: "done" })}
        subagents={[child("call-task-1", { status: "done" })]}
        onStopSubagent={() => undefined}
      />,
    );
    expect(settled).not.toContain("subagent-stop");

    // No stop handler at all: the running row stays read-only.
    const readOnly = render(
      <SubagentTopology
        tool={parentTask()}
        subagents={[child("call-task-1")]}
      />,
    );
    expect(readOnly).not.toContain("subagent-stop");
  });

  it("renders the plural root label for a multi-child fan-out", () => {
    // Today's runtime keys one child per `task` call, but the card reports the
    // count it is actually handed rather than assuming the invariant.
    const html = render(
      <SubagentTopology
        tool={parentTask()}
        subagents={[
          child("call-task-1"),
          child("call-task-1", { status: "done", eventCount: 7 }),
        ]}
      />,
    );
    expect(html).toContain("正在编排 2 个代理");
    expect(html).toContain("7 步");
  });
});

describe("subagentRowLabel", () => {
  it("falls back to the call id when neither agent nor prompt is usable", () => {
    expect(subagentRowLabel(parentTask({ args: undefined }))).toBe("call-task-1");
    expect(subagentRowLabel(parentTask({ args: {} }))).toBe("call-task-1");
  });

  it("trims the prompt to its first line", () => {
    expect(
      subagentRowLabel(
        parentTask({ args: { prompt: "  first line\nsecond line  " } }),
      ),
    ).toBe("first line");
  });
});
