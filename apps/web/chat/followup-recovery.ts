import type {
  ProductControlStatus,
  ProductMessage,
} from "../product/product-api-types";

/**
 * Recovery for a successor the server abandoned (productization F.5).
 *
 * When the API cannot start a claimed successor it eventually releases the
 * claim and escalates: the control becomes `abandoned`, the session becomes
 * `needs_attention`, and the ledger message becomes `needs_attention` carrying
 * the server's reason (`abandon_failed_followup_start` in `apps/api/src/lib.rs`).
 * The user's way back is `POST
 * /product/sessions/{session_id}/controls/{control_id}/confirm`, which returns
 * the control to `pending`, moves the session back to `idle`, and re-attempts
 * the start. The message and the control share one id, so the confirm names the
 * ledger row the reader is looking at.
 */

/** The dictionary lookup the shell's components receive from `useCopy`. */
export type CopyLookup = (
  path: string,
  params?: Record<string, string | number>,
) => string;

/**
 * Whether one ledger row is a stranded successor this client may ask the server
 * to retry.
 *
 * `needs_attention` alone is not enough. A *successor* message (the control
 * kind `followup`) is recoverable, and so is a message created while the session
 * already needed attention — the store writes those `abandoned` with the reason
 * "session requires an explicit recovery decision", and the same route accepts
 * them. A *dropped interjection* (`requested_delivery === "current_run"`) also
 * reads as `needs_attention`, but its control kind is `steer` and the confirm
 * route refuses it ("only follow-up controls can be confirmed"); offering the
 * action there would be an action the server cannot honor, so it is not
 * offered — the row keeps its revoke affordance and nothing else.
 */
export function isStrandedSuccessor(message: ProductMessage): boolean {
  return (
    message.status === "needs_attention" &&
    message.requested_delivery === "successor"
  );
}

/**
 * One row's recovery state.
 *
 * `accepted` carries the status the server actually committed rather than a
 * boolean, because the route answers `pending` for a fresh confirmation (the
 * successor is back in the queue, and the API attempts its start before
 * answering) and returns the control unchanged on the idempotent path, where
 * `accepted`/`applied` means the retry is already being started. The two are
 * different facts and the row says which one it has.
 */
export type FollowupRecoveryOutcome =
  | { status: "idle" }
  | { status: "retrying" }
  | { status: "accepted"; controlStatus: ProductControlStatus }
  | { status: "failed"; reason: string };

export type FollowupRecoveryEvent =
  | { type: "retry" }
  | { type: "accepted"; controlStatus: ProductControlStatus }
  | { type: "failed"; reason: string };

export const IDLE_FOLLOWUP_RECOVERY: FollowupRecoveryOutcome = { status: "idle" };

/**
 * The row's own state machine.
 *
 * `retrying` is absorbing until a result arrives: a second request while one is
 * in flight would be a second server-side retry of the same successor, which is
 * exactly what the disabled action must prevent rather than what the server
 * should have to refuse.
 */
export function reduceFollowupRecovery(
  current: FollowupRecoveryOutcome,
  event: FollowupRecoveryEvent,
): FollowupRecoveryOutcome {
  switch (event.type) {
    case "retry":
      return current.status === "retrying" ? current : { status: "retrying" };
    case "accepted":
      return { status: "accepted", controlStatus: event.controlStatus };
    case "failed":
      return { status: "failed", reason: event.reason };
  }
}

/**
 * The retry action's label: the in-flight state is stated on the button the
 * reader pressed, which is also the control that is disabled while the request
 * is unsettled.
 */
export function followupRecoveryActionLabel(
  outcome: FollowupRecoveryOutcome,
  t: CopyLookup,
): string {
  return outcome.status === "retrying"
    ? t("chat.retryStrandedBusy")
    : t("chat.retryStranded");
}

/**
 * What the row says once the request has settled.
 *
 * Nothing is reported while the request is in flight (the button says so) or
 * before it was ever attempted, and every settled message is derived from the
 * server's answer: an accepted confirm names what the server committed, and a
 * refusal quotes the server's own reason instead of a generic failure.
 */
export function followupRecoveryNotice(
  outcome: FollowupRecoveryOutcome,
  t: CopyLookup,
): { tone: "ok" | "error"; text: string } | null {
  switch (outcome.status) {
    case "idle":
    case "retrying":
      return null;
    case "accepted":
      return {
        tone: "ok",
        text: t(
          outcome.controlStatus === "pending"
            ? "chat.retryStrandedAccepted"
            : "chat.retryStrandedStarting",
        ),
      };
    case "failed":
      return {
        tone: "error",
        text: t("chat.retryStrandedFailed", { reason: outcome.reason }),
      };
  }
}

/** The settled answer one confirm request produced. */
export type FollowupConfirmationResult =
  | { ok: true; controlStatus: ProductControlStatus }
  | { ok: false; reason: string };

/**
 * Session-level recovery: one action for every stranded successor in the
 * queue.
 *
 * A `needs_attention` session usually strands more than one row — every
 * successor enqueued behind the abandoned one is written `abandoned` too, and
 * an abandoned row is never drained on its own: each needs its own confirm
 * while the session holds no active claim. Confirming the whole set in one
 * pass is what lets the queue drain again: confirmed rows go back to
 * `pending`, and the server starts them in order as each turn finishes.
 *
 * The confirm route refuses while a turn is active
 * (`product_session_active`). Confirming the rows in a tight sequence mostly
 * lands ahead of the next claim, but a claim can win the race — the runner
 * retries that row briefly, then stops and reports what it did commit rather
 * than erroring the whole pass.
 */
export type FollowupBatchRecoveryOutcome =
  | { status: "idle" }
  | { status: "running"; total: number }
  | {
      /**
       * The pass ended: `remaining` rows still need a confirm. `reason` is the
       * server's own words for a refusal; `blockedByActiveTurn` instead means
       * the stop was a turn holding the session's claim past the retry window.
       */
      status: "settled";
      confirmed: number;
      remaining: number;
      reason: string | null;
      blockedByActiveTurn: boolean;
    };

export type FollowupBatchRecoveryEvent =
  | { type: "start"; total: number }
  | {
      type: "settled";
      confirmed: number;
      remaining: number;
      reason: string | null;
      blockedByActiveTurn: boolean;
    };

export const IDLE_FOLLOWUP_BATCH_RECOVERY: FollowupBatchRecoveryOutcome = {
  status: "idle",
};

/**
 * The batch pass is absorbing while it runs: a second press must not start a
 * second sweep of the same confirms.
 */
export function reduceFollowupBatchRecovery(
  current: FollowupBatchRecoveryOutcome,
  event: FollowupBatchRecoveryEvent,
): FollowupBatchRecoveryOutcome {
  switch (event.type) {
    case "start":
      return current.status === "running"
        ? current
        : { status: "running", total: event.total };
    case "settled":
      return {
        status: "settled",
        confirmed: event.confirmed,
        remaining: event.remaining,
        reason: event.reason,
        blockedByActiveTurn: event.blockedByActiveTurn,
      };
  }
}

/** The settled answer the session-level recovery pass produced. */
export interface FollowupBatchRecoveryResult {
  confirmed: number;
  remaining: number;
  reason: string | null;
  blockedByActiveTurn: boolean;
}

/**
 * The ids of every stranded successor, in the order the rows are shown — the
 * same order the server will start them in once confirmed.
 */
export function strandedFollowupIds(
  messages: readonly ProductMessage[],
): string[] {
  return messages
    .filter(isStrandedSuccessor)
    .slice()
    .sort(
      (left, right) =>
        (left.queue_order ?? left.seq) - (right.queue_order ?? right.seq) ||
        left.seq - right.seq,
    )
    .map((message) => message.id);
}

/** The session-level action's label: the in-flight pass is stated on it. */
export function followupBatchActionLabel(
  outcome: FollowupBatchRecoveryOutcome,
  t: CopyLookup,
): string {
  return outcome.status === "running"
    ? t("chat.recoverAllBusy")
    : t("chat.recoverAll");
}

/**
 * What the session-level action reports once the pass settles.
 *
 * Every line is derived from what the server committed: a full pass says the
 * queue is draining, a partial pass says how many remain (a turn claimed the
 * session mid-pass — the rest confirm on the next press), and a refusal quotes
 * the server's reason.
 */
export function followupBatchNotice(
  outcome: FollowupBatchRecoveryOutcome,
  t: CopyLookup,
): { tone: "ok" | "error"; text: string } | null {
  if (outcome.status !== "settled") {
    return null;
  }
  if (outcome.remaining === 0 && outcome.reason === null) {
    return {
      tone: "ok",
      text:
        outcome.confirmed === 1
          ? t("chat.recoverAllDoneOne")
          : t("chat.recoverAllDone", { count: outcome.confirmed }),
    };
  }
  // A pass that stopped early always carries the server's reason, or the fact
  // that a turn holds the session's claim. The line says how many did commit
  // so the reader knows the press was not lost.
  const countParams = {
    confirmed: outcome.confirmed,
    remaining: outcome.remaining,
  };
  if (outcome.reason !== null) {
    return {
      tone: "error",
      text:
        outcome.confirmed > 0
          ? t("chat.recoverAllPartial", { ...countParams, reason: outcome.reason })
          : t("chat.recoverAllFailed", { reason: outcome.reason }),
    };
  }
  if (outcome.blockedByActiveTurn) {
    return {
      tone: "error",
      text: t("chat.recoverAllActive", countParams),
    };
  }
  return {
    tone: "error",
    text: t("chat.recoverAllStopped", countParams),
  };
}
