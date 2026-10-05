import type { TranscriptRunGroup } from "../lib/rove-state";

/**
 * Resolve a message-ledger entry to the identity the transcript can reveal
 * (R7's "hit → jump" half).
 *
 * The ledger sequence a hit carries is *not* a transcript identity. The
 * transcript's own identities are projection-local and change with how the run
 * arrived: a turn restored from canonical events is rendered as
 * `message:<runId>:user`, while a turn sent in this browser session keeps the
 * optimistic id it was created with. What both cases share is the run: the
 * ledger message records the run it was delivered to, and the timeline holds that
 * run's user turn under whatever id it was projected with.
 *
 * So the run is the join, and the function returns the id of that run's user turn
 * — or null when the ledger message has no run yet (it is still queued) or the
 * transcript does not hold the run. Null is the answer the search overlay must be
 * able to show: claiming a jump the transcript cannot make would be a
 * fabrication.
 */

/** The canonical projection identity of a restored run's user turn. */
export function canonicalUserMessageId(runId: string): string {
  return `message:${runId}:user`;
}

export interface LocatableMessage {
  /** The run the message was delivered to; absent while it is still queued. */
  run_id?: string;
}

/**
 * The transcript message id to reveal, or null when the transcript holds no
 * revealable turn for this ledger message.
 */
export function transcriptLocateTarget(input: {
  timeline: readonly TranscriptRunGroup[];
  message: LocatableMessage;
}): string | null {
  const runId = input.message.run_id;
  if (runId === undefined) {
    return null;
  }
  const group = input.timeline.find((candidate) => candidate.runId === runId);
  if (group === undefined) {
    return null;
  }
  for (const item of group.items) {
    if (item.kind === "message" && item.message.role === "user") {
      return item.message.id;
    }
  }
  return null;
}