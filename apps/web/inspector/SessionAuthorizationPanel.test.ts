import { describe, expect, it } from "vitest";

import type { ProductTranscriptResponse } from "../product/product-api-types";
import { collectSessionAuthorizationRecords } from "./SessionAuthorizationPanel";

function segment(overrides: {
  ordinal: number;
  runId: string;
  inherited?: boolean;
  events: Array<{ seq: number; event: unknown }>;
}) {
  return {
    binding: {
      product_session_id: "ps-1",
      ordinal: overrides.ordinal,
      runtime_session_id: "rs-1",
      runtime_job_id: "job-1",
      runtime_run_id: overrides.runId,
      bound_at: "2026-09-17T00:00:00Z",
    },
    inherited: overrides.inherited ?? false,
    run_status: "done" as const,
    observed_through_seq: 0,
    last_event_seq: 0,
    events: overrides.events,
  } as ProductTranscriptResponse["segments"][number];
}

function transcript(
  segments: ProductTranscriptResponse["segments"],
): ProductTranscriptResponse {
  return {
    product_session_id: "ps-1",
    workspace_id: "pw-1",
    status: "complete",
    partial_reasons: [],
    segments,
  } as ProductTranscriptResponse;
}

function approvalNeeded(callId: string, name: string, seq: number) {
  return {
    seq,
    event: {
      type: "tool_call_approval_needed",
      call_id: callId,
      name,
      args: { path: "notes.md" },
      reason: "destructive tool requires explicit approval",
    },
  };
}

describe("collectSessionAuthorizationRecords", () => {
  it("projects the durable request side of every authorization request", () => {
    const records = collectSessionAuthorizationRecords(
      transcript([
        segment({
          ordinal: 1,
          runId: "run-a",
          events: [
            approvalNeeded("call-1", "write_file", 3),
            { seq: 4, event: { type: "llm_message", full: "hi" } },
          ],
        }),
      ]),
    );

    expect(records).toEqual([
      {
        callId: "call-1",
        toolName: "write_file",
        runId: "run-a",
        runOrdinal: 1,
        seq: 3,
        reason: "destructive tool requires explicit approval",
        inherited: false,
      },
    ]);
  });

  it("keeps only the first occurrence when an event is replayed", () => {
    const records = collectSessionAuthorizationRecords(
      transcript([
        segment({
          ordinal: 1,
          runId: "run-a",
          events: [approvalNeeded("call-1", "write_file", 3)],
        }),
        segment({
          ordinal: 2,
          runId: "run-b",
          events: [approvalNeeded("call-1", "write_file", 3)],
        }),
      ]),
    );

    expect(records).toHaveLength(1);
    expect(records[0]?.runId).toBe("run-a");
  });

  it("orders records newest-first and marks inherited runs", () => {
    const records = collectSessionAuthorizationRecords(
      transcript([
        segment({
          ordinal: 2,
          runId: "run-b",
          inherited: true,
          events: [approvalNeeded("call-2", "shell", 9)],
        }),
        segment({
          ordinal: 1,
          runId: "run-a",
          events: [approvalNeeded("call-1", "write_file", 3)],
        }),
      ]),
    );

    expect(records.map((record) => record.callId)).toEqual(["call-2", "call-1"]);
    expect(records[0]).toMatchObject({ runId: "run-b", inherited: true });
    expect(records[1]).toMatchObject({ runId: "run-a", inherited: false });
  });

  it("returns nothing for a session with no authorization requests", () => {
    const records = collectSessionAuthorizationRecords(
      transcript([
        segment({
          ordinal: 1,
          runId: "run-a",
          events: [{ seq: 1, event: { type: "llm_message", full: "hi" } }],
        }),
      ]),
    );

    expect(records).toEqual([]);
  });

  it("bounds the section so one session cannot flood the panel", () => {
    const events = Array.from({ length: 80 }, (_unused, index) =>
      approvalNeeded(`call-${index}`, "write_file", index + 1),
    );
    const records = collectSessionAuthorizationRecords(
      transcript([segment({ ordinal: 1, runId: "run-a", events })]),
    );

    expect(records).toHaveLength(50);
    // Newest-first, so the bound keeps the most recent requests.
    expect(records[0]?.seq).toBe(80);
  });
});
