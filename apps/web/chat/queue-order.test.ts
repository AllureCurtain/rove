import { describe, expect, it } from "vitest";

import type { ProductMessage } from "../product/product-api-types";
import {
  actionableQueueOrder,
  isQueueReorderable,
  reorderedQueueIds,
  successorQueue,
} from "./queue-order";

function queued(
  id: string,
  seq: number,
  overrides: Partial<ProductMessage> = {},
): ProductMessage {
  return {
    id,
    product_session_id: "s1",
    content: id,
    requested_delivery: "successor",
    status: "queued",
    seq,
    created_at: "2026-09-26T00:00:00.000Z",
    ...overrides,
  };
}

describe("successorQueue", () => {
  it("keeps ledger order for messages that were never moved", () => {
    const queue = successorQueue([queued("b", 2), queued("a", 1)]);
    expect(queue.map((message) => message.id)).toEqual(["a", "b"]);
  });

  it("orders by the explicit queue position and falls back to seq", () => {
    // A reorder writes queue_order only; the ledger seq stays as it was.
    const queue = successorQueue([
      queued("a", 1, { queue_order: 2 }),
      queued("b", 2, { queue_order: 0 }),
      queued("c", 3),
    ]);
    expect(queue.map((message) => message.id)).toEqual(["b", "a", "c"]);
  });

  it("excludes interjections and non-queued messages", () => {
    const queue = successorQueue([
      queued("queued", 1),
      queued("steer", 2, { requested_delivery: "current_run" }),
      queued("claimed", 3, { status: "claimed_successor" }),
      queued("revoked", 4, { status: "revoked" }),
    ]);
    expect(queue.map((message) => message.id)).toEqual(["queued"]);
  });
});

describe("reorderedQueueIds", () => {
  it("returns the whole queue with the two neighbours exchanged", () => {
    const messages = [queued("a", 1), queued("b", 2), queued("c", 3)];
    expect(reorderedQueueIds(messages, "b", "up")).toEqual(["b", "a", "c"]);
    expect(reorderedQueueIds(messages, "b", "down")).toEqual(["a", "c", "b"]);
  });

  it("sends the queue it can see, not a partial move request", () => {
    // The endpoint rejects a list that does not cover the whole queue, so the
    // list must always be the complete successor queue.
    const messages = [
      queued("a", 1, { queue_order: 1 }),
      queued("b", 2, { queue_order: 0 }),
      queued("steer", 3, { requested_delivery: "current_run" }),
    ];
    expect(reorderedQueueIds(messages, "a", "up")).toEqual(["a", "b"]);
  });

  it("refuses a move that has nothing to swap with", () => {
    const messages = [queued("a", 1), queued("b", 2)];
    expect(reorderedQueueIds(messages, "a", "up")).toBeNull();
    expect(reorderedQueueIds(messages, "b", "down")).toBeNull();
  });

  it("refuses an unknown or non-queue message", () => {
    const messages = [
      queued("a", 1),
      queued("b", 2),
      queued("steer", 3, { requested_delivery: "current_run" }),
    ];
    expect(reorderedQueueIds(messages, "missing", "up")).toBeNull();
    expect(reorderedQueueIds(messages, "steer", "up")).toBeNull();
  });
});

describe("isQueueReorderable", () => {
  it("accepts only queued successors", () => {
    expect(isQueueReorderable(queued("a", 1))).toBe(true);
    expect(isQueueReorderable(queued("a", 1, { requested_delivery: "current_run" }))).toBe(false);
    expect(isQueueReorderable(queued("a", 1, { status: "intervention_requested" }))).toBe(false);
    expect(isQueueReorderable(queued("a", 1, { status: "needs_attention" }))).toBe(false);
  });
});

describe("actionableQueueOrder", () => {
  it("shows a moved message at its queue position, not its ledger position", () => {
    // The list route returns `seq` order and a reorder never rewrites `seq`, so
    // without this the bubbles would stay where they were created while the
    // server dispatched them in the other order.
    const messages = [
      queued("a", 1, { queue_order: 2 }),
      queued("b", 2, { queue_order: 1 }),
    ];
    expect(actionableQueueOrder(messages).map((message) => message.id)).toEqual([
      "b",
      "a",
    ]);
  });

  it("keeps creation order for everything that was never moved", () => {
    const messages = [
      queued("z", 30),
      queued("a", 10, { requested_delivery: "current_run" }),
      queued("m", 20, { status: "needs_attention" }),
    ];
    expect(actionableQueueOrder(messages).map((message) => message.id)).toEqual([
      "a",
      "m",
      "z",
    ]);
  });

  it("drops rows that are already delivered, withdrawn, or claimed", () => {
    const messages = [
      queued("queued", 1),
      queued("revoked", 2, { status: "revoked" }),
      queued("claimed", 3, { status: "claimed_successor" }),
      queued("applied", 4, { status: "applied_current_run" }),
    ];
    expect(actionableQueueOrder(messages).map((message) => message.id)).toEqual([
      "queued",
    ]);
  });
});
