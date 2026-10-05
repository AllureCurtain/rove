import { describe, expect, it } from "vitest";

import type { ProductMessage } from "../product/product-api-types";
import { branchPointForTurn, ledgerMessageForTurn } from "./branch-at-message";

function user(
  id: string,
  seq: number,
  runId: string | undefined,
  content = id,
): ProductMessage {
  return {
    id,
    product_session_id: "s1",
    content,
    requested_delivery: "successor",
    status: "claimed_successor",
    seq,
    created_at: "2026-09-26T00:00:00.000Z",
    ...(runId === undefined ? {} : { run_id: runId }),
  };
}

describe("ledgerMessageForTurn", () => {
  it("matches on both the delivered run and the content", () => {
    const messages = [user("m1", 1, "run-1"), user("m2", 2, "run-2")];
    expect(
      ledgerMessageForTurn({ messages, content: "m2", runId: "run-2" })?.id,
    ).toBe("m2");
  });

  it("refuses when the same text was sent twice in one run", () => {
    // Two identical sends in one run share content; picking either would fork
    // from an arbitrary prefix, so the branch point stays unavailable.
    const messages = [
      user("m1", 1, "run-1", "same"),
      user("m2", 2, "run-1", "same"),
    ];
    expect(ledgerMessageForTurn({ messages, content: "same", runId: "run-1" })).toBeNull();
  });

  it("refuses a message another run owns", () => {
    const messages = [user("m1", 1, "run-1")];
    expect(ledgerMessageForTurn({ messages, content: "m1", runId: "run-2" })).toBeNull();
  });

  it("refuses without a run id or with empty content", () => {
    const messages = [user("m1", 1, "run-1")];
    expect(ledgerMessageForTurn({ messages, content: "m1", runId: null })).toBeNull();
    expect(ledgerMessageForTurn({ messages, content: "", runId: "run-1" })).toBeNull();
  });

  it("refuses a ledger message that was never delivered to a run", () => {
    const messages = [user("m1", 1, undefined)];
    expect(ledgerMessageForTurn({ messages, content: "m1", runId: "run-1" })).toBeNull();
  });
});

describe("branchPointForTurn", () => {
  it("offers the branch only while the parent can fork", () => {
    const messages = [user("m1", 1, "run-1")];
    expect(
      branchPointForTurn({ messages, content: "m1", runId: "run-1", forkAvailable: true })?.seq,
    ).toBe(1);
    expect(
      branchPointForTurn({ messages, content: "m1", runId: "run-1", forkAvailable: false }),
    ).toBeNull();
  });
});
