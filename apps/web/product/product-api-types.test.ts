import { describe, expect, it } from "vitest";

import {
  ProductApiSchemaError,
  parseProductSession,
  parseStreamEvent,
} from "./product-api-types";

/**
 * The tool result envelope is where the runtime publishes its own measurement
 * of a call. These cases pin the narrow projection the shell reads: the timing
 * and protocol identity survive parsing, and anything malformed is a schema
 * error instead of a value a component has to defend against.
 */

function completedEvent(protocolMetadata: unknown) {
  return {
    type: "tool_call_completed",
    call_id: "call-1",
    result: {
      call_id: "call-1",
      output: "notes.md read",
      metadata: {
        status: "ok",
        risk_level: "low",
        read_only: true,
        affected_paths: ["notes.md"],
        workspace_changed: false,
        diff_summary: [],
      },
      envelope: {
        outcome: "success",
        summary_text: "notes.md read",
        protocol_metadata: protocolMetadata,
      },
    },
  };
}

describe("tool result protocol metadata", () => {
  it("keeps the published duration and protocol identity", () => {
    const event = parseStreamEvent(
      completedEvent({
        protocol: "mcp",
        server_config_id: "server-1",
        protocol_version: "2025-06-18",
        remote_tool_name: "read",
        attempt_count: 2,
        duration_ms: 2_400,
      }),
    );
    expect(event.type).toBe("tool_call_completed");
    if (event.type !== "tool_call_completed") {
      throw new Error("expected a completed tool call");
    }
    expect(event.result.envelope?.protocol_metadata).toEqual({
      protocol: "mcp",
      server_config_id: "server-1",
      protocol_version: "2025-06-18",
      remote_tool_name: "read",
      attempt_count: 2,
      duration_ms: 2_400,
    });
  });

  it("drops keys the shell does not read", () => {
    const event = parseStreamEvent(
      completedEvent({ protocol: "mcp", session_hash: "opaque", duration_ms: 10 }),
    );
    if (event.type !== "tool_call_completed") {
      throw new Error("expected a completed tool call");
    }
    expect(event.result.envelope?.protocol_metadata).toEqual({
      protocol: "mcp",
      duration_ms: 10,
    });
  });

  it("rejects a duration that is not a whole non-negative number", () => {
    expect(() => parseStreamEvent(completedEvent({ duration_ms: -1 }))).toThrow(
      ProductApiSchemaError,
    );
    expect(() => parseStreamEvent(completedEvent({ duration_ms: 1.5 }))).toThrow(
      ProductApiSchemaError,
    );
    expect(() => parseStreamEvent(completedEvent({ duration_ms: "2400" }))).toThrow(
      ProductApiSchemaError,
    );
  });

  it("rejects a protocol identity that is not a bounded string", () => {
    expect(() => parseStreamEvent(completedEvent({ protocol: 7 }))).toThrow(
      ProductApiSchemaError,
    );
    expect(() =>
      parseStreamEvent(completedEvent({ protocol: "mcp\u0000spoof" })),
    ).toThrow(ProductApiSchemaError);
  });

  it("accepts a result without any protocol metadata", () => {
    const event = parseStreamEvent({
      type: "tool_call_completed",
      call_id: "call-1",
      result: { call_id: "call-1", output: "ok" },
    });
    if (event.type !== "tool_call_completed") {
      throw new Error("expected a completed tool call");
    }
    expect(event.result.envelope).toBeUndefined();
  });
});

/**
 * R3 (migration 016) adds the outcome of the most recently finished turn. The
 * status field cannot express it, so the shell must be able to tell "the last
 * turn succeeded" from "this session never ran", and must not invent an answer
 * from a half-written payload.
 */
function sessionRecord(extra: Record<string, unknown> = {}) {
  return {
    id: "sess-1",
    workspace_id: "ws-1",
    title: "Session",
    status: "idle",
    created_at: "2026-09-26T00:00:00.000Z",
    updated_at: "2026-09-26T00:00:00.000Z",
    ...extra,
  };
}

describe("product session outcome", () => {
  it("leaves the outcome absent for a session that never ran", () => {
    const session = parseProductSession(sessionRecord());

    expect(session.last_outcome).toBeUndefined();
    expect(session.last_outcome_at).toBeUndefined();
  });

  it("reads every published outcome with its timestamp", () => {
    for (const outcome of ["success", "failed", "cancelled"] as const) {
      const session = parseProductSession(
        sessionRecord({
          last_outcome: outcome,
          last_outcome_at: "2026-09-26T12:00:00.000Z",
        }),
      );

      expect(session.last_outcome).toBe(outcome);
      expect(session.last_outcome_at).toBe("2026-09-26T12:00:00.000Z");
    }
  });

  it("treats explicit nulls as no outcome", () => {
    const session = parseProductSession(
      sessionRecord({ last_outcome: null, last_outcome_at: null }),
    );

    expect(session.last_outcome).toBeUndefined();
    expect(session.last_outcome_at).toBeUndefined();
  });

  it("rejects an outcome this client does not know", () => {
    expect(() =>
      parseProductSession(
        sessionRecord({
          last_outcome: "timed_out",
          last_outcome_at: "2026-09-26T12:00:00.000Z",
        }),
      ),
    ).toThrow(ProductApiSchemaError);
  });

  it("rejects a payload with only one half of the outcome", () => {
    expect(() =>
      parseProductSession(sessionRecord({ last_outcome: "success" })),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseProductSession(
        sessionRecord({ last_outcome_at: "2026-09-26T12:00:00.000Z" }),
      ),
    ).toThrow(ProductApiSchemaError);
  });
});
