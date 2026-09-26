import { describe, expect, it } from "vitest";

import type { TranscriptTimelineEntry, TranscriptTimelineItem } from "../lib/rove-state";
import { shouldRestoreOnStop } from "./smart-stop";

function entry(id: string, inherited = false): TranscriptTimelineEntry {
  return { id, kind: "message", entityId: id, runId: null, runOrdinal: null, eventSeq: null, inherited, sourceSessionId: null };
}

function user(id: string, inherited = false): TranscriptTimelineItem {
  return { entry: entry(id, inherited), kind: "message", message: { id, role: "user", content: id, status: "final" } };
}

function assistant(id: string): TranscriptTimelineItem {
  return { entry: entry(id), kind: "message", message: { id, role: "assistant", content: id, status: "final" } };
}

function tool(id: string): TranscriptTimelineItem {
  return { entry: entry(id), kind: "tool", tool: { id, name: id, status: "running", details: "" } };
}

describe("shouldRestoreOnStop", () => {
  it("restores when the last user message has no reply yet", () => {
    expect(shouldRestoreOnStop([user("m1")])).toBe(true);
  });

  it("does not restore once a tool call or an answer exists", () => {
    expect(shouldRestoreOnStop([user("m1"), tool("t1")])).toBe(false);
    expect(shouldRestoreOnStop([user("m1"), assistant("a1")])).toBe(false);
  });

  it("does not restore an inherited message", () => {
    expect(shouldRestoreOnStop([user("m1", true)])).toBe(false);
  });

  it("does not restore when there is no user message", () => {
    expect(shouldRestoreOnStop([assistant("a1")])).toBe(false);
  });
});
