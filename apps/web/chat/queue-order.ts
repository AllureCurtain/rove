import type { ProductMessage } from "../product/product-api-types";

/**
 * The successor queue in delivery order (R4).
 *
 * `queue_order` is the explicit position a reorder or a `successor` promotion
 * writes; `seq` is the ledger order, which no reorder rewrites. A message that
 * has never been moved keeps `undefined` and therefore keeps its creation
 * order, so the two keys are read exactly the way the server reads them:
 * `queue_order ?? seq`, then `seq` as the tie-break.
 */
export function successorQueue(
  messages: readonly ProductMessage[],
): ProductMessage[] {
  return messages
    .filter(
      (message) =>
        message.status === "queued" && message.requested_delivery === "successor",
    )
    .slice()
    .sort(
      (left, right) =>
        (left.queue_order ?? left.seq) - (right.queue_order ?? right.seq) ||
        left.seq - right.seq,
    );
}

/**
 * The whole ordered id list a single adjacent swap sends to the reorder
 * endpoint.
 *
 * The endpoint takes the session's *entire* successor queue and refuses a
 * partial or stale list instead of merging it, so the UI cannot send "move this
 * one up": it sends the queue it can see, with the two neighbours exchanged.
 * Returns null when the move is not possible (unknown message, already at the
 * end it is moving towards), so the caller does not invent an order.
 */
export function reorderedQueueIds(
  messages: readonly ProductMessage[],
  messageId: string,
  direction: "up" | "down",
): string[] | null {
  const queue = successorQueue(messages);
  const index = queue.findIndex((message) => message.id === messageId);
  if (index < 0) {
    return null;
  }
  const target = direction === "up" ? index - 1 : index + 1;
  if (target < 0 || target >= queue.length) {
    return null;
  }
  const ids = queue.map((message) => message.id);
  const moved = ids[index]!;
  ids[index] = ids[target]!;
  ids[target] = moved;
  return ids;
}

/**
 * The actionable ledger rows in the order the transcript shows them.
 *
 * The ledger list route orders by `seq` alone — a reorder rewrites `queue_order`
 * and never `seq` — so the position key has to be applied here as well: without
 * it a moved message keeps its creation position on screen while the server
 * dispatches it from its new one. Only rows a reorder or a promotion actually
 * moved carry `queue_order`, so everything else keeps its `seq` order.
 */
export function actionableQueueOrder(
  messages: readonly ProductMessage[],
): ProductMessage[] {
  return messages
    .filter(
      (message) =>
        message.status === "queued" ||
        message.status === "intervention_requested" ||
        message.status === "needs_attention",
    )
    .slice()
    .sort(
      (left, right) =>
        (left.queue_order ?? left.seq) - (right.queue_order ?? right.seq) ||
        left.seq - right.seq,
    );
}

/**
 * Whether one queued message may take part in the successor queue at all.
 *
 * Only queued successors do: a `current_run` interjection is not part of the
 * queue, and every other status is either already delivered, withdrawn, or
 * waiting on a human, and the endpoint refuses all of them.
 */
export function isQueueReorderable(message: ProductMessage): boolean {
  return (
    message.status === "queued" && message.requested_delivery === "successor"
  );
}
