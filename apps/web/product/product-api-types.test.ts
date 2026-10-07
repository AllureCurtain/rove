import { describe, expect, it } from "vitest";

import {
  ProductApiSchemaError,
  parseStreamEvent,
} from "./product-api-types";
import { readPromptBuildMetadata, readPromptPruningFacts } from "../lib/rove-types";

/**
 * `parseStreamEvent` is the SSE frame boundary: the wire is untrusted input,
 * so the guard proves the frame is an object whose `type` is a canonical event
 * kind before the generated `StreamEvent` union describes it. Field shape is
 * pinned by the spec and `tests/event_contract.rs`, not re-validated here —
 * the tests below pin only what the guard itself still answers.
 */

describe("stream event frame guard", () => {
  it("accepts a canonical event kind and returns the frame unchanged", () => {
    const frame = {
      type: "tool_call_completed",
      call_id: "call-1",
      result: { call_id: "call-1", output: "ok" },
    };

    expect(parseStreamEvent(frame)).toBe(frame);
  });

  it("rejects a frame that is not an object", () => {
    for (const frame of [null, 7, "run_started", [1, 2]]) {
      expect(() => parseStreamEvent(frame)).toThrow(ProductApiSchemaError);
    }
  });

  it("rejects a frame without a canonical event kind", () => {
    for (const frame of [
      {},
      { type: 7 },
      { type: "" },
      { type: "run_startd" },
      { type: "RunStarted" },
      { type: "not_an_event" },
    ]) {
      expect(() => parseStreamEvent(frame)).toThrow(ProductApiSchemaError);
    }
  });

  it("lets payload fields pass through untouched", () => {
    // The guard does not project the payload: whatever the server published is
    // what the consumer reads, including fields this build does not know.
    const frame = {
      type: "provider_retry",
      attempt: 2,
      max_attempts: 4,
      delay_ms: 2_000,
      reason: "transient:request_failed",
      phase: "model_call",
      future_field: { nested: true },
    };

    expect(parseStreamEvent(frame)).toBe(frame);
  });
});

/**
 * The pruning facts are additive fields on the metadata the runtime already
 * publishes, and they are omitted while zero. Both arrival paths decode the
 * same wire record through `readPromptBuildMetadata`/`readPromptPruningFacts`:
 * a build that pruned nothing keeps the shape it had before, and a build that
 * pruned something is read without inventing the fields it left out. Records
 * outside the wire bounds are withheld — the live path renders the transcript
 * without the facts rather than failing the frame over them.
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

  function metadataOf(metadata: Record<string, unknown> = {}) {
    const decoded = readPromptBuildMetadata({ ...baseMetadata, ...metadata });
    if (decoded === null) {
      throw new Error("expected decodable prompt build metadata");
    }
    return decoded;
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
    expect(
      metadataOf({ pruning_policy: "rove.pruning.v2" }).pruning?.pruning_policy,
    ).toBe("rove.pruning.v2");
  });

  it("withholds facts the server cannot publish", () => {
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
      expect(
        readPromptBuildMetadata({ ...baseMetadata, ...metadata })?.pruning,
        JSON.stringify(metadata),
      ).toBeUndefined();
    }
  });

  it("bounds the digest list well above the policy cap", () => {
    const digest = (index: number) => `call-${index}:sha256:${"a".repeat(8)}`;
    expect(
      readPromptBuildMetadata({
        ...baseMetadata,
        pruned_tool_results: 1,
        pruned_payload_digests: Array.from({ length: 64 }, (_, index) => digest(index)),
      })?.pruning,
    ).toBeDefined();
    expect(
      readPromptBuildMetadata({
        ...baseMetadata,
        pruned_tool_results: 1,
        pruned_payload_digests: Array.from({ length: 65 }, (_, index) => digest(index)),
      })?.pruning,
    ).toBeUndefined();
  });

  it("withholds an out-of-bounds fact set instead of refusing the frame", () => {
    // The live path has only a frame and must keep rendering, so a record
    // outside the bounds answers the same absence the refusal produced: the
    // metadata decodes as no facts, not as a partial measurement.
    const digest = (index: number) => `call-${index}:sha256:${"a".repeat(8)}`;
    const tooMany = Array.from({ length: 65 }, (_, index) => digest(index));
    expect(
      readPromptPruningFacts({ pruned_tool_results: 1, pruned_payload_digests: tooMany }),
    ).toBeUndefined();

    const longPolicy = "p".repeat(129);
    expect(
      readPromptPruningFacts({ pruned_tool_results: 1, pruning_policy: longPolicy }),
    ).toBeUndefined();

    for (const unusable of [
      { pruned_payload_digests: ["   "] },
      { pruned_payload_digests: ["call-1:sha256:aa\u0000"] },
      { pruning_policy: "   " },
      { pruning_policy: `rove.pruning.v1` },
      { pruned_tool_results: 1.5 },
    ]) {
      expect(readPromptPruningFacts(unusable), JSON.stringify(unusable)).toBeUndefined();
    }

    // An optional member is still a member of the record: an unusable
    // `prompt_cache_key` means the whole record is not the shape the decoder
    // answers for, so the build is withheld rather than returned without it.
    for (const unusable of [
      { pruning_policy: "rove.pruning.v1", prompt_cache_key: "   " },
      { pruned_tool_results: 0, prompt_cache_key: 7 },
    ]) {
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
});
