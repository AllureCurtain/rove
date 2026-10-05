import { describe, expect, it } from "vitest";

import type {
  ChatMessage,
  TranscriptTimelineEntry,
  TranscriptTimelineItem,
} from "../lib/rove-state";
import {
  isPartialAbort,
  salvagedPartialAfter,
  shouldRestoreOnStop,
  stopBoundary,
} from "./smart-stop";

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

describe("partial abort branch (runtime R2b)", () => {
  it("needs both the marker and work that was already produced", () => {
    expect(isPartialAbort({ aborted: true, producedWork: true })).toBe(true);
    expect(isPartialAbort({ aborted: true, producedWork: false })).toBe(false);
    expect(isPartialAbort({ aborted: false, producedWork: true })).toBe(false);
    expect(isPartialAbort({ aborted: undefined, producedWork: true })).toBe(false);
  });
});

function message(overrides: Partial<ChatMessage> & { id: string }): ChatMessage {
  return {
    role: "assistant",
    content: "text",
    status: "final",
    ...overrides,
  };
}

describe("salvagedPartialAfter", () => {
  it("reports only a message that arrived after the stop boundary", () => {
    const settled = [message({ id: "a1" })];
    const boundary = stopBoundary(settled);
    const salvaged = message({ id: "a2", aborted: true });
    expect(salvagedPartialAfter([...settled, salvaged], boundary)).toBe(salvaged);
  });

  it("does not fire for an old aborted message from an earlier stop", () => {
    // A restored transcript can already hold a salvaged partial from an earlier
    // cancelled turn; refreshing or switching back must not re-announce it.
    const old = message({ id: "a1", aborted: true });
    const settled = [old, message({ id: "a2", content: "complete" })];
    expect(salvagedPartialAfter(settled, stopBoundary(settled))).toBeNull();
  });

  it("does not fire when the stopped turn kept nothing", () => {
    const settled = [message({ id: "a1", content: "" })];
    const boundary = stopBoundary(settled);
    expect(
      salvagedPartialAfter([...settled, message({ id: "a2", aborted: true, content: "" })], boundary),
    ).toBeNull();
  });

  it("ignores an unmarked message and a user message", () => {
    const boundary = stopBoundary([]);
    expect(salvagedPartialAfter([message({ id: "a1" })], boundary)).toBeNull();
    expect(
      salvagedPartialAfter(
        [message({ id: "m1", role: "user", aborted: true })],
        boundary,
      ),
    ).toBeNull();
  });
});
