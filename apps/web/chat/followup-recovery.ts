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
