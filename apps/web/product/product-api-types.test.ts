import { describe, expect, it } from "vitest";

import { ProductApiSchemaError, parseStreamEvent } from "./product-api-types";

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
