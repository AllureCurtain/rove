import { describe, expect, it } from "vitest";

import type { ChatMessage, SubagentView, ToolCallView } from "../lib/rove-state";
import {
  buildSubagentTranscript,
  subagentMessageOrder,
  taskCallArgs,
} from "./subagent-transcript";

function parentTask(args: unknown): ToolCallView {
  return {
    id: "call-task-1",
    name: "task",
    status: "running",
    details: "",
    args,
  };
}

function subagent(overrides: Partial<SubagentView> = {}): SubagentView {
  return {
    parentCallId: "call-task-1",
    status: "running",
    eventCount: 0,
    messages: [],
    tools: [],
    ...overrides,
  };
}

function message(
  id: string,
  content: string,
  overrides: Partial<ChatMessage> = {},
): ChatMessage {
  return {
    id,
    role: "assistant",
    content,
    status: "final",
    ...overrides,
  };
}

function tool(id: string, name = "read_file"): ToolCallView {
  return { id, name, status: "done", details: "" };
}

describe("taskCallArgs", () => {
  it("reads prompt and agent from a populated task call", () => {
    expect(
      taskCallArgs(parentTask({ prompt: "survey", agent: "explorer" })),
    ).toEqual({ prompt: "survey", agent: "explorer" });
  });

  it("tolerates missing or malformed args", () => {
    expect(taskCallArgs(undefined)).toEqual({ prompt: "" });
    expect(taskCallArgs(parentTask(undefined))).toEqual({ prompt: "" });
    expect(taskCallArgs(parentTask("not-an-object"))).toEqual({ prompt: "" });
    expect(taskCallArgs(parentTask({ agent: 42 }))).toEqual({ prompt: "" });
  });
});

describe("subagentMessageOrder", () => {
  it("parses the wrapped-event ordinal out of the child message id", () => {
    expect(
      subagentMessageOrder(
        message("message:sub:call-task-1:assistant:7", "x"),
      ),
    ).toBe(7);
  });

  it("falls back for ids without an ordinal suffix", () => {
    expect(subagentMessageOrder(message("bare-id", "x"))).toBe(
      Number.MAX_SAFE_INTEGER,
    );
  });
});

describe("buildSubagentTranscript", () => {
  it("returns null when there is no delegation", () => {
    expect(buildSubagentTranscript(undefined, parentTask({ prompt: "x" }))).toBeNull();
  });

  it("opens the turn with the parent's task prompt as the user row", () => {
    const transcript = buildSubagentTranscript(
      subagent(),
      parentTask({ prompt: "Survey the repo", agent: "explorer" }),
    );
    expect(transcript).not.toBeNull();
    expect(transcript!.turns).toHaveLength(1);
    expect(transcript!.turns[0]!.prompt).toBe("Survey the repo");
    expect(transcript!.turns[0]!.agent).toBe("explorer");
  });

  it("maps child messages and tools into rows preserving each kind's arrival order", () => {
    const transcript = buildSubagentTranscript(
      subagent({
        messages: [
          message("message:sub:call-task-1:assistant:2", "first"),
          message("message:sub:call-task-1:assistant:9", "second"),
        ],
        // The projection prepends tools, so the stored order is newest-first.
        tools: [tool("call-b", "write_file"), tool("call-a", "read_file")],
      }),
      parentTask({ prompt: "do work" }),
    );
    const rows = transcript!.turns[0]!.rows;
    const messageRows = rows.filter((row) => row.kind === "message");
    const toolRows = rows.filter((row) => row.kind === "tool");
    expect(messageRows.map((row) => row.kind === "message" && row.message.content)).toEqual([
      "first",
      "second",
    ]);
    // Reversed back into started-arrival order: call-a before call-b.
    expect(toolRows.map((row) => row.kind === "tool" && row.tool.id)).toEqual([
      "call-a",
      "call-b",
    ]);
    expect(rows).toHaveLength(4);
  });

  it("keeps an empty delegation honest — a turn with no rows yet", () => {
    const transcript = buildSubagentTranscript(
      subagent(),
      parentTask({ prompt: "waiting" }),
    );
    expect(transcript!.turns[0]!.rows).toHaveLength(0);
  });
});
