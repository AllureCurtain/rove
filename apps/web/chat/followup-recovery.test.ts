import { describe, expect, it } from "vitest";

import { createTranslator } from "../copy";
import type { ProductMessage } from "../product/product-api-types";
import {
  IDLE_FOLLOWUP_BATCH_RECOVERY,
  IDLE_FOLLOWUP_RECOVERY,
  followupBatchActionLabel,
  followupBatchNotice,
  followupRecoveryActionLabel,
  followupRecoveryNotice,
  isStrandedSuccessor,
  reduceFollowupBatchRecovery,
  reduceFollowupRecovery,
  strandedFollowupIds,
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

describe("strandedFollowupIds", () => {
  it("lists only the stranded successors, in dispatch order", () => {
    const messages = [
      // `queue_order` wins over `seq`, exactly as the server dispatches it.
      message({
        id: "second-stranded",
        seq: 1,
        queue_order: 2,
      }),
      message({
        id: "first-stranded",
        seq: 2,
        queue_order: 1,
      }),
      // A dropped interjection is `needs_attention` but not a successor: the
      // confirm route refuses it, so it is not part of the pass.
      message({
        id: "stranded-interjection",
        requested_delivery: "current_run",
        seq: 3,
      }),
      message({ id: "queued-successor", status: "queued", seq: 4 }),
    ];
    expect(strandedFollowupIds(messages)).toEqual([
      "first-stranded",
      "second-stranded",
    ]);
  });

  it("is empty when nothing is stranded", () => {
    expect(
      strandedFollowupIds([message({ status: "queued" })]),
    ).toEqual([]);
  });
});

describe("reduceFollowupBatchRecovery", () => {
  it("moves from idle to running on the action", () => {
    expect(
      reduceFollowupBatchRecovery(IDLE_FOLLOWUP_BATCH_RECOVERY, {
        type: "start",
        total: 3,
      }),
    ).toEqual({ status: "running", total: 3 });
  });

  it("keeps the in-flight pass absorbing", () => {
    const running = { status: "running", total: 2 } as const;
    expect(
      reduceFollowupBatchRecovery(running, { type: "start", total: 2 }),
    ).toBe(running);
  });

  it("records what the pass committed, including a partial stop", () => {
    expect(
      reduceFollowupBatchRecovery(
        { status: "running", total: 3 },
        {
          type: "settled",
          confirmed: 1,
          remaining: 2,
          reason: "busy",
          blockedByActiveTurn: false,
        },
      ),
    ).toEqual({
      status: "settled",
      confirmed: 1,
      remaining: 2,
      reason: "busy",
      blockedByActiveTurn: false,
    });
  });
});

describe("followupBatchActionLabel", () => {
  it("states the in-flight pass on the disabled action", () => {
    expect(
      followupBatchActionLabel({ status: "running", total: 2 }, zh),
    ).toBe("正在恢复…");
    expect(
      followupBatchActionLabel({ status: "running", total: 2 }, en),
    ).toBe("Recovering…");
  });

  it("otherwise offers the session recovery", () => {
    expect(followupBatchActionLabel(IDLE_FOLLOWUP_BATCH_RECOVERY, zh)).toBe(
      "恢复会话",
    );
    expect(followupBatchActionLabel(IDLE_FOLLOWUP_BATCH_RECOVERY, en)).toBe(
      "Recover session",
    );
  });
});

describe("followupBatchNotice", () => {
  it("says nothing before the pass has an answer", () => {
    expect(followupBatchNotice(IDLE_FOLLOWUP_BATCH_RECOVERY, zh)).toBeNull();
    expect(
      followupBatchNotice({ status: "running", total: 2 }, zh),
    ).toBeNull();
  });

  it("reports a full pass by count", () => {
    expect(
      followupBatchNotice(
        {
          status: "settled",
          confirmed: 3,
          remaining: 0,
          reason: null,
          blockedByActiveTurn: false,
        },
        zh,
      ),
    ).toEqual({
      tone: "ok",
      text: "会话已恢复——服务器已重新排队 3 条消息。",
    });
    expect(
      followupBatchNotice(
        {
          status: "settled",
          confirmed: 1,
          remaining: 0,
          reason: null,
          blockedByActiveTurn: false,
        },
        en,
      ),
    ).toEqual({
      tone: "ok",
      text: "Session recovered — the server re-queued this message.",
    });
  });

  it("reports a partial pass with the server's reason and what did commit", () => {
    const notice = followupBatchNotice(
      {
        status: "settled",
        confirmed: 1,
        remaining: 2,
        reason: "only an abandoned follow-up can be confirmed",
        blockedByActiveTurn: false,
      },
      en,
    );
    expect(notice?.tone).toBe("error");
    expect(notice?.text).toContain("Recovered 1");
    expect(notice?.text).toContain("2");
    expect(notice?.text).toContain("abandoned follow-up");
  });

  it("reports a stop on the session's active claim in its own words", () => {
    // A real turn can hold the claim for minutes: the pass names that as the
    // stop, not as a server refusal, and the button stays offered for the rest.
    const notice = followupBatchNotice(
      {
        status: "settled",
        confirmed: 1,
        remaining: 2,
        reason: null,
        blockedByActiveTurn: true,
      },
      zh,
    );
    expect(notice?.tone).toBe("error");
    expect(notice?.text).toContain("已恢复 1 条");
    expect(notice?.text).toContain("2 条");
    expect(notice?.text).toContain("正在运行");
  });

  it("reports a pass that confirmed nothing as an outright failure", () => {
    expect(
      followupBatchNotice(
        {
          status: "settled",
          confirmed: 0,
          remaining: 2,
          reason: "refused",
          blockedByActiveTurn: false,
        },
        zh,
      ),
    ).toEqual({ tone: "error", text: "恢复失败：refused" });
  });

  it("reports a pass stopped without a server reason in neutral terms", () => {
    // The focused session moved mid-pass: the rows that did commit still count,
    // and the line must not quote a refusal the server never gave.
    const notice = followupBatchNotice(
      {
        status: "settled",
        confirmed: 2,
        remaining: 1,
        reason: null,
        blockedByActiveTurn: false,
      },
      en,
    );
    expect(notice?.tone).toBe("error");
    expect(notice?.text).toContain("Recovered 2");
    expect(notice?.text).toContain("1");
    expect(notice?.text).not.toContain("null");
  });
});
