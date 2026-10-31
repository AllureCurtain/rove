import { renderToStaticMarkup } from "react-dom/server";
import { createElement } from "react";
import { describe, expect, it } from "vitest";

import { CopyProvider } from "../copy/CopyProvider";
import {
  createWorkbenchState,
  type SubagentView,
  type ToolCallView,
} from "../lib/rove-state";
import { SubagentsPanel } from "./SubagentsPanel";

function parentTask(overrides: Partial<ToolCallView> = {}): ToolCallView {
  return {
    id: "call-task-1",
    name: "task",
    status: "running",
    details: "",
    args: { prompt: "Survey the repo", agent: "explorer" },
    ...overrides,
  };
}

function child(overrides: Partial<SubagentView> = {}): SubagentView {
  return {
    parentCallId: "call-task-1",
    status: "running",
    eventCount: 4,
    messages: [
      {
        id: "message:sub:call-task-1:assistant:2",
        role: "assistant",
        content: "partial answer",
        status: "streaming",
      },
    ],
    tools: [
      {
        id: "child-call-1",
        name: "read_file",
        status: "done",
        details: "done",
      },
    ],
    ...overrides,
  };
}

function render(element: React.ReactElement) {
  return renderToStaticMarkup(createElement(CopyProvider, null, element));
}

describe("SubagentsPanel", () => {
  it("shows the empty state when the run delegated nothing", () => {
    const html = render(
      <SubagentsPanel runState={createWorkbenchState()} />,
    );
    expect(html).toContain("inspector-empty");
    expect(html).toContain("暂无子代理");
  });

  it("lists delegations and renders the selected child's mini transcript", () => {
    const state = {
      ...createWorkbenchState(),
      activeJobId: "job-1",
      tools: [parentTask()],
      subagents: [child()],
    };
    const html = render(
      <SubagentsPanel runState={state} onStopSubagent={() => undefined} />,
    );
    // The list names the requested agent and its status.
    expect(html).toContain("subagent-panel__list");
    expect(html).toContain("explorer");
    // The turn opens with the parent task prompt as the user row.
    expect(html).toContain("Survey the repo");
    // The child's own work renders in the transcript's language.
    expect(html).toContain("tool-card");
    expect(html).toContain("partial answer");
    // The read-only foot states who drives the delegate; a running child stops.
    expect(html).toContain("由主代理驱动");
    expect(html).toContain("subagent-stop");
    expect(html).toContain(">停止<");
  });

  it("keeps the foot read-only with no stop once the child settles", () => {
    const state = {
      ...createWorkbenchState(),
      tools: [parentTask({ status: "done" })],
      subagents: [child({ status: "done" })],
    };
    const html = render(
      <SubagentsPanel runState={state} onStopSubagent={() => undefined} />,
    );
    expect(html).toContain("由主代理驱动");
    expect(html).not.toContain("subagent-stop");
    expect(html).not.toContain("正在停止");
  });
});
