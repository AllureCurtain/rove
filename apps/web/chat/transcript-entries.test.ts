import { describe, expect, it } from "vitest";

import type {
  ChatMessage,
  ToolCallView,
  TranscriptInputView,
  TranscriptTimelineEntry,
  TranscriptTimelineItem,
} from "../lib/rove-state";
import { activityWantsOpen, buildTranscriptEntries } from "./transcript-entries";

function entry(id: string): TranscriptTimelineEntry {
  return {
    id,
    kind: "message",
    entityId: id,
    runId: "run-1",
    runOrdinal: 1,
    eventSeq: 1,
    inherited: false,
    sourceSessionId: null,
  };
}

function message(id: string, role: ChatMessage["role"], status: ChatMessage["status"] = "final"): ChatMessage {
  return { id, role, content: id, status };
}

function tool(id: string, status: ToolCallView["status"]): ToolCallView {
  return { id, name: id, status, details: "" };
}

function input(id: string, status: TranscriptInputView["status"]): TranscriptInputView {
  return { id, timelineId: id, prompt: id, status };
}

function item(partial: TranscriptTimelineItem): TranscriptTimelineItem {
  return partial;
}

describe("buildTranscriptEntries", () => {
  it("folds consecutive tools and inputs into one activity", () => {
    const entries = buildTranscriptEntries([
      item({ entry: entry("m1"), kind: "message", message: message("m1", "user") }),
      item({ entry: entry("t1"), kind: "tool", tool: tool("t1", "done") }),
      item({ entry: entry("i1"), kind: "input", input: input("i1", "submitted") }),
      item({ entry: entry("t2"), kind: "tool", tool: tool("t2", "done") }),
      item({ entry: entry("m2"), kind: "message", message: message("m2", "assistant") }),
    ]);
    expect(entries.map((value) => value.kind)).toEqual(["turn", "activity", "answer"]);
    const activity = entries[1]!;
    expect(activity.kind === "activity" ? activity.tools.map((value) => value.id) : []).toEqual([
      "t1",
      "t2",
    ]);
  });

  it("starts a new activity after a user message", () => {
    const entries = buildTranscriptEntries([
      item({ entry: entry("t1"), kind: "tool", tool: tool("t1", "done") }),
      item({ entry: entry("m1"), kind: "message", message: message("m1", "user") }),
      item({ entry: entry("t2"), kind: "tool", tool: tool("t2", "done") }),
    ]);
    expect(entries.map((value) => value.kind)).toEqual(["activity", "turn", "activity"]);
  });

  it("marks only the trailing streaming answer as streaming", () => {
    const entries = buildTranscriptEntries([
      item({ entry: entry("m1"), kind: "message", message: message("m1", "assistant", "streaming") }),
      item({ entry: entry("m2"), kind: "message", message: message("m2", "assistant", "streaming") }),
    ]);
    expect(entries.map((value) => (value.kind === "answer" ? value.streaming : null))).toEqual([
      false,
      true,
    ]);
  });

  it("uses the first item's id so the row identity survives re-renders", () => {
    const entries = buildTranscriptEntries([
      item({ entry: entry("t1"), kind: "tool", tool: tool("t1", "done") }),
      item({ entry: entry("t2"), kind: "tool", tool: tool("t2", "done") }),
    ]);
    expect(entries[0]?.id).toBe("t1");
  });
});

describe("activityWantsOpen", () => {
  it("is open while a tool is running and closed once everything settled", () => {
    const [running] = buildTranscriptEntries([
      item({ entry: entry("t1"), kind: "tool", tool: tool("t1", "running") }),
    ]);
    const [done] = buildTranscriptEntries([
      item({ entry: entry("t1"), kind: "tool", tool: tool("t1", "done") }),
    ]);
    expect(running?.kind === "activity" && activityWantsOpen(running)).toBe(true);
    expect(done?.kind === "activity" && activityWantsOpen(done)).toBe(false);
  });
});
