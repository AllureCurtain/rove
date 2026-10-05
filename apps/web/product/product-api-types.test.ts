import { describe, expect, it } from "vitest";

import {
  MAX_MESSAGE_ATTACHMENTS,
  ProductApiSchemaError,
  parseCreateProductControlRequest,
  parseProductAttachmentUpload,
  parseProductMessage,
  parseProductMessageAttachmentRef,
  parseProductSession,
  parseStreamEvent,
} from "./product-api-types";
import { readPromptBuildMetadata, readPromptPruningFacts } from "../lib/rove-types";

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
 * The retry notice is a runtime fact the shell only projects, so parsing must
 * keep the attempt accounting intact and refuse values that could not have come
 * from the runtime.
 */
describe("provider retry notices", () => {
  function retryEvent(overrides: Record<string, unknown> = {}) {
    return {
      type: "provider_retry",
      attempt: 2,
      max_attempts: 4,
      delay_ms: 2_000,
      reason: "transient:request_failed",
      phase: "model_call",
      ...overrides,
    };
  }

  it("keeps the runtime's attempt accounting and whitelisted reason", () => {
    const event = parseStreamEvent(retryEvent());

    expect(event).toEqual(retryEvent());
  });

  it("rejects an attempt that could not come from the runtime", () => {
    expect(() => parseStreamEvent(retryEvent({ attempt: 0 }))).toThrow(
      ProductApiSchemaError,
    );
    expect(() => parseStreamEvent(retryEvent({ attempt: 1.5 }))).toThrow(
      ProductApiSchemaError,
    );
    expect(() => parseStreamEvent(retryEvent({ max_attempts: 0 }))).toThrow(
      ProductApiSchemaError,
    );
    expect(() => parseStreamEvent(retryEvent({ delay_ms: -1 }))).toThrow(
      ProductApiSchemaError,
    );
  });

  it("rejects a reason or phase that is not a bounded string", () => {
    expect(() => parseStreamEvent(retryEvent({ reason: "" }))).toThrow(
      ProductApiSchemaError,
    );
    expect(() => parseStreamEvent(retryEvent({ phase: 7 }))).toThrow(
      ProductApiSchemaError,
    );
    expect(() =>
      parseStreamEvent(retryEvent({ reason: "leak\u0000spoof" })),
    ).toThrow(ProductApiSchemaError);
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

const attachmentRef = {
  attachment_id: "01J00000000000000000000031",
  content_type: "image/png",
  size: 2048,
  sha256: "a".repeat(64),
  name: "shot.png",
  availability: "available",
};

describe("message attachment references", () => {
  function message(overrides: Record<string, unknown> = {}) {
    return {
      id: "01J00000000000000000000032",
      product_session_id: "01J00000000000000000000002",
      content: "look at this",
      requested_delivery: "successor",
      status: "queued",
      seq: 1,
      created_at: "2026-09-26T12:00:00.000Z",
      ...overrides,
    };
  }

  it("leaves attachments absent on a message that carries none", () => {
    expect(parseProductMessage(message()).attachments).toBeUndefined();
    // An empty array is the same fact as an absent field, not a difference.
    expect(parseProductMessage(message({ attachments: [] })).attachments).toBeUndefined();
  });

  it("reads every published field of a reference", () => {
    const parsed = parseProductMessage(message({ attachments: [attachmentRef] }));
    expect(parsed.attachments).toEqual([attachmentRef]);
  });

  it("carries the degradation reason without inventing one", () => {
    const plain = parseProductMessageAttachmentRef(attachmentRef);
    expect(plain.degradation).toBeUndefined();
    const degraded = parseProductMessageAttachmentRef({
      ...attachmentRef,
      degradation: "not sent: the selected model does not accept image input",
    });
    expect(degraded.degradation).toContain("does not accept image input");
  });

  it("refuses an availability or a size the server cannot publish", () => {
    expect(() =>
      parseProductMessageAttachmentRef({ ...attachmentRef, availability: "gone" }),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseProductMessageAttachmentRef({ ...attachmentRef, size: -1 }),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseProductMessageAttachmentRef({ ...attachmentRef, sha256: "" }),
    ).toThrow(ProductApiSchemaError);
  });

  it("bounds the reference list", () => {
    expect(() =>
      parseProductMessage(
        message({ attachments: Array.from({ length: MAX_MESSAGE_ATTACHMENTS + 1 }, () => attachmentRef) }),
      ),
    ).toThrow(ProductApiSchemaError);
  });

  it("sends attachments only when the request actually has some", () => {
    expect(
      parseCreateProductControlRequest({ content: "hello" }).attachments,
    ).toBeUndefined();
    expect(
      parseCreateProductControlRequest({ content: "hello", attachments: [] }).attachments,
    ).toBeUndefined();
    expect(
      parseCreateProductControlRequest({
        content: "hello",
        attachments: [{ attachment_id: attachmentRef.attachment_id, name: "shot.png" }],
      }).attachments,
    ).toEqual([{ attachment_id: attachmentRef.attachment_id, name: "shot.png" }]);
  });

  it("refuses an attachment request field the server does not accept", () => {
    expect(() =>
      parseCreateProductControlRequest({
        content: "hello",
        attachments: [{ attachment_id: attachmentRef.attachment_id, path: "C:\\x.png" }],
      }),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseCreateProductControlRequest({
        content: "hello",
        attachments: [{ attachment_id: "" }],
      }),
    ).toThrow(ProductApiSchemaError);
  });

  it("reads an upload response with the verified values and warnings", () => {
    const parsed = parseProductAttachmentUpload({
      attachment_id: attachmentRef.attachment_id,
      product_session_id: "01J00000000000000000000002",
      content_type: "application/pdf",
      size: 4096,
      sha256: "b".repeat(64),
      name: "brief.pdf",
      status: "staged",
      expires_at: "2026-09-26T13:00:00.000Z",
      warnings: ["possible_secret_content"],
    });
    expect(parsed).toEqual({
      attachment_id: attachmentRef.attachment_id,
      product_session_id: "01J00000000000000000000002",
      content_type: "application/pdf",
      size: 4096,
      sha256: "b".repeat(64),
      name: "brief.pdf",
      status: "staged",
      expires_at: "2026-09-26T13:00:00.000Z",
      warnings: ["possible_secret_content"],
    });
  });

  it("requires the warnings array an upload always publishes", () => {
    expect(() =>
      parseProductAttachmentUpload({
        attachment_id: attachmentRef.attachment_id,
        product_session_id: "01J00000000000000000000002",
        content_type: "image/png",
        size: 1,
        sha256: "c".repeat(64),
        status: "staged",
      }),
    ).toThrow(ProductApiSchemaError);
    expect(() =>
      parseProductAttachmentUpload({
        attachment_id: attachmentRef.attachment_id,
        product_session_id: "01J00000000000000000000002",
        content_type: "image/png",
        size: 1,
        sha256: "c".repeat(64),
        status: "attached",
        warnings: [],
      }),
    ).toThrow(ProductApiSchemaError);
  });
});

/**
 * The pruning facts are additive fields on the metadata the runtime already
 * publishes, and they are omitted while zero. These cases pin both halves of
 * that contract: a build that pruned nothing keeps the shape it had before, and
 * a build that pruned something is read without inventing the fields it left out.
 */
describe("prompt build pruning facts", () => {
  const baseMetadata = {
    prompt_hash: "sha256:prompt",
    stable_prefix_hash: "sha256:prefix",
    workspace_fingerprint: "sha256:workspace",
    tool_signature: "sha256:tools",
    token_estimate: 512,
    included_history_messages: 6,
    dropped_history_messages: 4,
  };

  function promptBuilt(metadata: Record<string, unknown> = {}) {
    return parseStreamEvent({
      type: "prompt_built",
      metadata: { ...baseMetadata, ...metadata },
    });
  }

  function metadataOf(metadata: Record<string, unknown> = {}) {
    const event = promptBuilt(metadata);
    if (event.type !== "prompt_built") {
      throw new Error(`expected a prompt_built event, got ${event.type}`);
    }
    return event.metadata;
  }

  it("leaves pruning absent on a build that pruned nothing", () => {
    const metadata = metadataOf();
    expect(metadata.pruning).toBeUndefined();
    expect(Object.keys(metadata)).not.toContain("pruning");
  });

  it("reads every recorded fact when the request pruned something", () => {
    expect(
      metadataOf({
        pruned_tool_results: 3,
        pruned_payload_bytes: 90_000,
        pruned_excerpt_bytes: 6_000,
        pruned_omitted_messages: 12,
        pruned_omitted_bytes: 44_000,
        pruned_payload_digests: ["call-9:sha256:aaaa", "call-4:sha256:bbbb"],
        pruning_policy: "rove.pruning.v1",
      }).pruning,
    ).toEqual({
      pruned_tool_results: 3,
      pruned_payload_bytes: 90_000,
      pruned_excerpt_bytes: 6_000,
      pruned_omitted_messages: 12,
      pruned_omitted_bytes: 44_000,
      pruned_payload_digests: ["call-9:sha256:aaaa", "call-4:sha256:bbbb"],
      pruning_policy: "rove.pruning.v1",
    });
  });

  it("fills in the facts the server omitted instead of reporting them as sent", () => {
    // The wire drops each field while it is zero, so a fact set that is present
    // at all can still name only one of them.
    expect(metadataOf({ pruned_omitted_messages: 2 }).pruning).toEqual({
      pruned_tool_results: 0,
      pruned_payload_bytes: 0,
      pruned_excerpt_bytes: 0,
      pruned_omitted_messages: 2,
      pruned_omitted_bytes: 0,
      pruned_payload_digests: [],
      pruning_policy: null,
    });
  });

  it("does not read an empty digest list as a pruning fact set", () => {
    // `pruned_payload_digests` is `skip_serializing_if = "Vec::is_empty"`, so the
    // runtime never publishes an empty list: an empty array is the same wire
    // state as an absent key, and a fact set built from it would be all zeroes.
    expect(metadataOf({ pruned_payload_digests: [] }).pruning).toBeUndefined();
  });

  it("keeps a policy version this build has never seen", () => {
    // A revised policy is a real pruning event; the reader is allowed to say it
    // does not recognize the rule, not to drop the fact.
    expect(metadataOf({ pruning_policy: "rove.pruning.v2" }).pruning?.pruning_policy).toBe(
      "rove.pruning.v2",
    );
  });

  it("refuses facts the server cannot publish", () => {
    const refused: Record<string, unknown>[] = [
      { pruned_tool_results: -1 },
      { pruned_payload_bytes: 1.5 },
      { pruned_omitted_bytes: "many" },
      { pruned_payload_digests: "call-1:sha256:aaaa" },
      { pruned_payload_digests: [""] },
      { pruned_payload_digests: ["call-1:sha256:aa\u0000"] },
      { pruning_policy: "" },
      { pruning_policy: 7 },
    ];
    for (const metadata of refused) {
      expect(() => promptBuilt(metadata), JSON.stringify(metadata)).toThrow(
        ProductApiSchemaError,
      );
    }
  });

  it("bounds the digest list well above the policy cap", () => {
    const digest = (index: number) => `call-${index}:sha256:${"a".repeat(8)}`;
    expect(() =>
      promptBuilt({
        pruned_tool_results: 1,
        pruned_payload_digests: Array.from({ length: 64 }, (_, index) => digest(index)),
      }),
    ).not.toThrow();
    expect(() =>
      promptBuilt({
        pruned_tool_results: 1,
        pruned_payload_digests: Array.from({ length: 65 }, (_, index) => digest(index)),
      }),
    ).toThrow(ProductApiSchemaError);
  });

  it("bounds the live path with the same numbers, and withholds instead of refusing", () => {
    // The deliberate asymmetry, pinned: the restored path has a schema and refuses
    // a record outside these bounds, while the live path has only a frame and must
    // keep rendering, so it answers the same absence the refusal produces. Both use
    // one set of bounds and the same predicates, so a raised cap cannot make the
    // live path render what the restored path refuses.
    const digest = (index: number) => `call-${index}:sha256:${"a".repeat(8)}`;
    const tooMany = Array.from({ length: 65 }, (_, index) => digest(index));
    expect(() => promptBuilt({ pruned_tool_results: 1, pruned_payload_digests: tooMany }))
      .toThrow(ProductApiSchemaError);
    expect(
      readPromptPruningFacts({ pruned_tool_results: 1, pruned_payload_digests: tooMany }),
    ).toBeUndefined();

    const longPolicy = "p".repeat(129);
    expect(() => promptBuilt({ pruned_tool_results: 1, pruning_policy: longPolicy }))
      .toThrow(ProductApiSchemaError);
    expect(
      readPromptPruningFacts({ pruned_tool_results: 1, pruning_policy: longPolicy }),
    ).toBeUndefined();

    // The predicates the schema applies beyond the bounds are the live path's too,
    // which is what keeps a record that parses from being the only one that renders.
    for (const unusable of [
      { pruned_payload_digests: ["   "] },
      { pruned_payload_digests: ["call-1:sha256:aa\u0000"] },
      { pruning_policy: "   " },
      { pruning_policy: `rove.pruning.v1\u007f` },
      { pruned_tool_results: 1.5 },
    ]) {
      expect(() => promptBuilt(unusable), JSON.stringify(unusable)).toThrow(
        ProductApiSchemaError,
      );
      expect(readPromptPruningFacts(unusable), JSON.stringify(unusable)).toBeUndefined();
    }

    // An optional member is still a member of the record: the schema refuses the
    // whole build over an unusable `prompt_cache_key`, so the live decoder must not
    // answer with the build and the pruning facts beside it either — that is the
    // one place where a member-by-member reading would describe a record the
    // restored path throws on.
    for (const unusable of [
      { pruning_policy: "rove.pruning.v1", prompt_cache_key: "   " },
      { pruned_tool_results: 0, prompt_cache_key: 7 },
    ]) {
      expect(() => promptBuilt(unusable), JSON.stringify(unusable)).toThrow(
        ProductApiSchemaError,
      );
      expect(
        readPromptBuildMetadata({ ...baseMetadata, ...unusable }),
        JSON.stringify(unusable),
      ).toBeNull();
    }
  });

  it("ignores a pruning field a later runtime adds", () => {
    // Forward compatibility is the point of the flat additive shape: this build
    // reads what it knows and does not fail the transcript over a new fact.
    const metadata = metadataOf({
      pruned_tool_results: 1,
      pruned_payload_bytes: 2_048,
      pruned_excerpt_bytes: 1_024,
      pruning_policy: "rove.pruning.v1",
      pruned_future_fact: 5,
    });
    expect(metadata.pruning?.pruned_tool_results).toBe(1);
  });

  it("decodes a live frame into exactly what this validated parse produces", () => {
    // The restored transcript is validated here; a live turn is decoded by
    // `readPromptBuildMetadata` in `lib/rove-state.ts`, which has no schema to
    // refuse the frame with. Both must land on the same object, or the evidence
    // panel would show a difference that is not in the data — the live turn
    // without its facts, the same turn after a reload with them.
    const accepted: Record<string, unknown>[] = [
      {},
      { pruned_omitted_messages: 12, pruned_omitted_bytes: 44_000, pruning_policy: "rove.pruning.v1" },
      {
        pruned_tool_results: 3,
        pruned_payload_bytes: 90_000,
        pruned_excerpt_bytes: 6_000,
        pruned_omitted_messages: 12,
        pruned_omitted_bytes: 44_000,
        pruned_payload_digests: ["call-9:sha256:aaaa", "call-4:sha256:bbbb"],
        pruning_policy: "rove.pruning.v1",
      },
      { pruning_policy: "rove.pruning.v2" },
      { pruned_payload_digests: [] },
      { prompt_cache_key: "sha256:cache" },
    ];
    for (const record of accepted) {
      const strict = metadataOf(record);
      const wire = { ...baseMetadata, ...record };
      expect(readPromptBuildMetadata(wire), JSON.stringify(record)).toEqual(strict);
    }
  });
});
