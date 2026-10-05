import { describe, expect, it } from "vitest";

import { createTranslator } from "../copy";
import type { ProductMessage } from "../product/product-api-types";
import {
  IDLE_FOLLOWUP_RECOVERY,
  followupRecoveryActionLabel,
  followupRecoveryNotice,
  isStrandedSuccessor,
  reduceFollowupRecovery,
  type FollowupRecoveryOutcome,
} from "./followup-recovery";

const zh = createTranslator("zh-CN");
const en = createTranslator("en-US");

function message(overrides: Partial<ProductMessage> = {}): ProductMessage {
  return {
    id: "message-1",
    product_session_id: "session-1",
    content: "queued work",
    requested_delivery: "successor",
    status: "needs_attention",
    seq: 1,
    created_at: "2026-09-26T00:00:00.000Z",
    ...overrides,
  };
}

describe("isStrandedSuccessor", () => {
  it("accepts an abandoned successor", () => {
    expect(isStrandedSuccessor(message())).toBe(true);
    expect(
      isStrandedSuccessor(
        message({ reason: "successor start failed 3 times; last failure: x" }),
      ),
    ).toBe(true);
  });

  it("refuses a row the confirm route would refuse", () => {
    // A dropped interjection is also `needs_attention`, but its control kind is
    // `steer`: the confirm route answers 409 for it, so the action is absent
    // rather than offered and refused.
    expect(
      isStrandedSuccessor(message({ requested_delivery: "current_run" })),
    ).toBe(false);
    for (const status of [
      "queued",
      "intervention_requested",
      "applied_current_run",
      "claimed_successor",
      "revoked",
    ] as const) {
      expect(isStrandedSuccessor(message({ status }))).toBe(false);
    }
  });
});

describe("reduceFollowupRecovery", () => {
  it("moves from idle to retrying on the action", () => {
    expect(reduceFollowupRecovery(IDLE_FOLLOWUP_RECOVERY, { type: "retry" })).toEqual({
      status: "retrying",
    });
  });

  it("keeps the in-flight state absorbing", () => {
    const retrying: FollowupRecoveryOutcome = { status: "retrying" };
    // A second press while the first request is unsettled must not start a
    // second server-side retry of the same successor.
    expect(reduceFollowupRecovery(retrying, { type: "retry" })).toBe(retrying);
  });

  it("records what the server committed, not that something succeeded", () => {
    const retrying: FollowupRecoveryOutcome = { status: "retrying" };
    expect(
      reduceFollowupRecovery(retrying, { type: "accepted", controlStatus: "pending" }),
    ).toEqual({ status: "accepted", controlStatus: "pending" });
    expect(
      reduceFollowupRecovery(retrying, { type: "accepted", controlStatus: "applied" }),
    ).toEqual({ status: "accepted", controlStatus: "applied" });
  });

  it("carries the server's refusal reason", () => {
    expect(
      reduceFollowupRecovery(
        { status: "retrying" },
        { type: "failed", reason: "an abandoned follow-up cannot be confirmed while a turn is active" },
      ),
    ).toEqual({
      status: "failed",
      reason: "an abandoned follow-up cannot be confirmed while a turn is active",
    });
  });

  it("lets a refused retry be attempted again", () => {
    const failed: FollowupRecoveryOutcome = { status: "failed", reason: "busy" };
    expect(reduceFollowupRecovery(failed, { type: "retry" })).toEqual({
      status: "retrying",
    });
  });
});

describe("followupRecoveryActionLabel", () => {
  it("states the in-flight request on the button that is disabled", () => {
    expect(followupRecoveryActionLabel({ status: "retrying" }, zh)).toBe("重试中…");
    expect(followupRecoveryActionLabel({ status: "retrying" }, en)).toBe("Retrying…");
  });

  it("otherwise offers the retry", () => {
    for (const outcome of [
      IDLE_FOLLOWUP_RECOVERY,
      { status: "accepted", controlStatus: "pending" },
      { status: "failed", reason: "busy" },
    ] satisfies FollowupRecoveryOutcome[]) {
      expect(followupRecoveryActionLabel(outcome, zh)).toBe("重试这条消息");
      expect(followupRecoveryActionLabel(outcome, en)).toBe("Retry this message");
    }
  });
});

describe("followupRecoveryNotice", () => {
  it("says nothing before the server has answered", () => {
    // Nothing may look successful before the server confirms it: an idle row and
    // an unsettled request both report no outcome.
    expect(followupRecoveryNotice(IDLE_FOLLOWUP_RECOVERY, zh)).toBeNull();
    expect(followupRecoveryNotice({ status: "retrying" }, zh)).toBeNull();
  });

  it("names the state the server committed", () => {
    expect(
      followupRecoveryNotice({ status: "accepted", controlStatus: "pending" }, zh),
    ).toEqual({ tone: "ok", text: "重试已被接受——服务器已把这条消息重新排队。" });
    // The idempotent path answers the control unchanged, because the retry is
    // already being started: a re-queue is not claimed where none happened.
    for (const controlStatus of ["accepted", "applied"] as const) {
      expect(followupRecoveryNotice({ status: "accepted", controlStatus }, zh)).toEqual({
        tone: "ok",
        text: "重试已被接受——服务器正在启动这条消息。",
      });
    }
  });

  it("quotes the server's refusal reason", () => {
    expect(
      followupRecoveryNotice(
        { status: "failed", reason: "an abandoned follow-up cannot be confirmed while a turn is active" },
        zh,
      ),
    ).toEqual({
      tone: "error",
      text: "重试失败：an abandoned follow-up cannot be confirmed while a turn is active",
    });
    expect(
      followupRecoveryNotice({ status: "failed", reason: "Failed to fetch" }, en),
    ).toEqual({ tone: "error", text: "Retry failed: Failed to fetch" });
  });
});
