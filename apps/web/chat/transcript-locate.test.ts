import { describe, expect, it } from "vitest";

import type { TranscriptRunGroup } from "../lib/rove-state";
import { transcriptLocateTarget } from "./transcript-locate";

/**
 * Hit → jump (R7).
 *
 * The ledger sequence is not a transcript identity: the same run's user turn is
 * keyed by its canonical projection id after a restore and by its optimistic id
 * after a send in this session. The run is the join, and a message that has no run
 * yet — or whose run the transcript does not hold — must be reported as
 * unlocatable instead of revealed by a jump that never happens.
 */

function group(input: {
  runId: string;
  messages?: Array<{ id: string; role: "user" | "assistant" }>;
  tools?: string[];
}): TranscriptRunGroup {
  return {
    id: `run:${input.runId}`,
    runId: input.runId,
    runOrdinal: 1,
    inherited: false,
    sourceSessionId: null,
    items: [
      ...(input.messages ?? []).map((message, index) => ({
        entry: {
          id: `${message.id}:entry`,
          kind: "message" as const,
          entityId: message.id,
          runId: input.runId,
          runOrdinal: 1,
          eventSeq: index + 1,
          inherited: false,
          sourceSessionId: null,
        },
        kind: "message" as const,
        message: {
          id: message.id,
          role: message.role,
          content: "text",
          status: "final" as const,
        },
      })),
      ...(input.tools ?? []).map((toolId, index) => ({
        entry: {
          id: `${toolId}:entry`,
          kind: "tool" as const,
          entityId: toolId,
          runId: input.runId,
          runOrdinal: 1,
          eventSeq: index + 1,
          inherited: false,
          sourceSessionId: null,
        },
        kind: "tool" as const,
        tool: { id: toolId, name: "fs.read", status: "done" as const } as never,
      })),
    ],
  };
}

describe("transcriptLocateTarget", () => {
  it("resolves a restored turn through its canonical projection id", () => {
    const timeline = [
      group({
        runId: "run-restored-1",
        messages: [{ id: "message:run-restored-1:user", role: "user" }],
      }),
    ];
    expect(
      transcriptLocateTarget({ timeline, message: { run_id: "run-restored-1" } }),
    ).toBe("message:run-restored-1:user");
  });

  it("resolves a turn sent in this session through its optimistic id", () => {
    // The same run, delivered live: the projection kept the id the optimistic
    // message was created with, so the canonical id would never match.
    const timeline = [
      group({
        runId: "run-live-1",
        messages: [{ id: "0f7c1f5e-optimistic", role: "user" }],
      }),
    ];
    expect(
      transcriptLocateTarget({ timeline, message: { run_id: "run-live-1" } }),
    ).toBe("0f7c1f5e-optimistic");
  });

  it("picks the user turn of the run, not its assistant reply", () => {
    const timeline = [
      group({
        runId: "run-2",
        messages: [
          { id: "assistant-first", role: "assistant" },
          { id: "user-turn", role: "user" },
        ],
      }),
    ];
    expect(
      transcriptLocateTarget({ timeline, message: { run_id: "run-2" } }),
    ).toBe("user-turn");
  });

  it("reports a queued message with no run yet as unlocatable", () => {
    const timeline = [
      group({ runId: "run-1", messages: [{ id: "user-turn", role: "user" }] }),
    ];
    expect(transcriptLocateTarget({ timeline, message: {} })).toBeNull();
  });

  it("reports a run the transcript does not hold as unlocatable", () => {
    const timeline = [
      group({ runId: "run-1", messages: [{ id: "user-turn", role: "user" }] }),
    ];
    // The ledger knows the turn, the loaded projection does not: no jump is
    // possible, so none may be claimed.
    expect(
      transcriptLocateTarget({ timeline, message: { run_id: "run-missing" } }),
    ).toBeNull();
  });

  it("reports a run without a projected user turn as unlocatable", () => {
    const timeline = [group({ runId: "run-1", tools: ["call-1"] })];
    expect(
      transcriptLocateTarget({ timeline, message: { run_id: "run-1" } }),
    ).toBeNull();
  });
});