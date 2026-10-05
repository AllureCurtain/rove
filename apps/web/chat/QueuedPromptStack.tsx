"use client";

import {
  ArrowDownIcon,
  ArrowUpIcon,
  Cross2Icon,
  RotateCounterClockwiseIcon,
} from "@radix-ui/react-icons";
import { useMemo, useReducer } from "react";

import { useCopy } from "../copy/CopyProvider";
import type {
  ProductMessage,
  ProductMessageAttachmentRef,
  ProductMessageDelivery,
} from "../product/product-api-types";
import { AttachmentRow } from "./Transcript";
import { actionableQueueOrder, successorQueue } from "./queue-order";
import {
  IDLE_FOLLOWUP_RECOVERY,
  followupRecoveryActionLabel,
  followupRecoveryNotice,
  isStrandedSuccessor,
  reduceFollowupRecovery,
  type FollowupConfirmationResult,
} from "./followup-recovery";

/**
 * §6.3: the server-persisted FIFO lives in the composer stack, above the
 * shell — it is where the next prompt waits, so it belongs next to the input
 * that adds to it rather than inside the transcript's history. The first
 * queued successor is the queue head: it reads with an accent edge and its
 * actions lock while a delivery request is in flight (`busy`).
 */
export function QueuedPromptStack({
  messages,
  busy = null,
  canPromote = false,
  editingMessageId = null,
  onPromote,
  onRevoke,
  onConfirmStranded,
  onEdit,
  onMove,
  loadAttachment,
}: {
  messages: readonly ProductMessage[];
  /** The control request in flight, if any — rows lock while one settles. */
  busy?: string | null;
  canPromote?: boolean;
  /** The queued message currently loaded into the composer for replacement. */
  editingMessageId?: string | null;
  onPromote?: (messageId: string, delivery: ProductMessageDelivery) => void;
  onRevoke?: (messageId: string) => void;
  /** F.5: ask the server to retry one successor it abandoned. */
  onConfirmStranded?: (
    messageId: string,
  ) => Promise<FollowupConfirmationResult>;
  /** Load a queued message back into the composer for replacement. */
  onEdit?: (messageId: string, content: string) => void;
  /** Adjacent swap within the successor queue. */
  onMove?: (messageId: string, direction: "up" | "down") => void;
  loadAttachment?: (attachmentId: string) => Promise<Blob>;
}) {
  const { t } = useCopy();
  // The ledger arrives in `seq` order, but a reorder rewrites `queue_order`
  // and never `seq`: the pending rows are shown in the order the server will
  // dispatch them, not the order they were created in.
  const actionableMessages = useMemo(
    () => actionableQueueOrder(messages),
    [messages],
  );
  // The successor queue in delivery order (`queue_order ?? seq`): adjacency
  // for move up/down, in the same order the server dispatches it. Its head is
  // the next-up row.
  const movableQueueIds = useMemo(
    () => successorQueue(messages).map((message) => message.id),
    [messages],
  );
  if (actionableMessages.length === 0) {
    return null;
  }
  const nextUpId = movableQueueIds[0] ?? null;
  return (
    <div className="composer-queue" role="list" aria-label={t("chat.queueTitle")}>
      {actionableMessages.map((message) => {
        const queueIndex = movableQueueIds.indexOf(message.id);
        return (
          <QueuedPromptRow
            key={message.id}
            message={message}
            attachments={message.attachments ?? []}
            loadAttachment={loadAttachment}
            busy={busy !== null}
            canPromote={canPromote}
            nextUp={message.id === nextUpId}
            editing={message.id === editingMessageId}
            movable={{
              up: queueIndex > 0,
              down: queueIndex >= 0 && queueIndex < movableQueueIds.length - 1,
            }}
            onPromote={onPromote ?? (() => {})}
            onRevoke={onRevoke ?? (() => {})}
            onConfirmStranded={onConfirmStranded}
            onEdit={onEdit}
            onMove={onMove}
          />
        );
      })}
    </div>
  );
}

function QueuedPromptRow({
  message,
  busy,
  canPromote,
  nextUp,
  editing,
  movable,
  attachments = [],
  loadAttachment,
  onPromote,
  onRevoke,
  onConfirmStranded,
  onEdit,
  onMove,
}: {
  message: ProductMessage;
  busy: boolean;
  canPromote: boolean;
  /** §6.3: the queue head — elevated surface, accent edge, locked while in flight. */
  nextUp: boolean;
  /** The message is loaded into the composer for replacement: lock other ops. */
  editing: boolean;
  movable: { up: boolean; down: boolean };
  attachments?: readonly ProductMessageAttachmentRef[];
  loadAttachment?: (attachmentId: string) => Promise<Blob>;
  onPromote: (messageId: string, delivery: ProductMessageDelivery) => void;
  onRevoke: (messageId: string) => void;
  onConfirmStranded?: (messageId: string) => Promise<FollowupConfirmationResult>;
  onEdit?: (messageId: string, content: string) => void;
  onMove?: (messageId: string, direction: "up" | "down") => void;
}) {
  const { t } = useCopy();
  const promotable = message.status === "queued" && canPromote;
  const revocable = message.status === "queued" || message.status === "needs_attention";
  // A stranded successor is always `needs_attention` and therefore already
  // `revocable`, so this flag only decides whether the retry is among the
  // actions — it does not open the actions row.
  const recoverable =
    onConfirmStranded !== undefined && isStrandedSuccessor(message);
  const [recovery, dispatchRecovery] = useReducer(
    reduceFollowupRecovery,
    IDLE_FOLLOWUP_RECOVERY,
  );
  const recoveryNotice = followupRecoveryNotice(recovery, t);
  const recoveryBusy = recovery.status === "retrying";
  // Only queued successor messages take part in edit and adjacent reorder; an
  // intervention targets the running turn and its order is not negotiable.
  const reorderable =
    message.status === "queued" && message.requested_delivery === "successor";
  const editable = reorderable && !editing;

  /**
   * The retry is dispatched before the request leaves and the answer is the only
   * thing that can settle it, so nothing is reported until the server has
   * answered (`apps/web/chat/followup-recovery.ts`).
   */
  async function retryStranded() {
    if (!onConfirmStranded || recovery.status === "retrying") {
      return;
    }
    dispatchRecovery({ type: "retry" });
    const result = await onConfirmStranded(message.id);
    dispatchRecovery(
      result.ok
        ? { type: "accepted", controlStatus: result.controlStatus }
        : { type: "failed", reason: result.reason },
    );
  }

  return (
    <article
      className="queued-message"
      data-status={message.status}
      data-delivery={message.requested_delivery}
      data-editing={editing || undefined}
      data-next-up={nextUp || undefined}
      role="listitem"
    >
      <div className="queued-message__row">
        <span className="queued-message__status">
          {messageStatusLabel(message, t)}
        </span>
        <p className="queued-message__text" title={message.content}>
          {message.content}
        </p>
        {promotable || revocable || editable ? (
          <div className="queued-message__actions">
            {editable && onEdit ? (
              <button
                type="button"
                className="secondary"
                disabled={busy}
                onClick={() => onEdit(message.id, message.content)}
              >
                {t("chat.queuedEdit")}
              </button>
            ) : null}
            {editable && onMove && movable.up ? (
              <button
                type="button"
                className="icon-button"
                disabled={busy}
                aria-label={t("chat.queuedMoveUp")}
                title={t("chat.queuedMoveUp")}
                onClick={() => onMove(message.id, "up")}
              >
                <ArrowUpIcon />
              </button>
            ) : null}
            {editable && onMove && movable.down ? (
              <button
                type="button"
                className="icon-button"
                disabled={busy}
                aria-label={t("chat.queuedMoveDown")}
                title={t("chat.queuedMoveDown")}
                onClick={() => onMove(message.id, "down")}
              >
                <ArrowDownIcon />
              </button>
            ) : null}
            {promotable ? (
              // Two delivery intents, stated explicitly: neither interrupts the
              // running turn by accident, and the interrupt is the one that says
              // so. The endpoint takes the intent in the body, so the buttons
              // differ only in the delivery they ask for.
              <>
                <button
                  type="button"
                  className="secondary"
                  disabled={busy}
                  title={t("chat.promoteSuccessorHint")}
                  onClick={() => onPromote(message.id, "successor")}
                >
                  <ArrowUpIcon />
                  {t("chat.promoteSuccessor")}
                </button>
                <button
                  type="button"
                  className="secondary"
                  disabled={busy}
                  title={t("chat.interjectHint")}
                  onClick={() => onPromote(message.id, "current_run")}
                >
                  {t("chat.interject")}
                </button>
              </>
            ) : null}
            {recoverable ? (
              // F.5: the successor was abandoned, so the reader's one action is
              // to ask the server to try it again. Disabled while the request
              // is unsettled — the label says which — and the result is the
              // server's, never an assumption.
              <button
                type="button"
                className="secondary"
                disabled={busy || recoveryBusy}
                aria-busy={recoveryBusy || undefined}
                title={t("chat.retryStrandedHint")}
                onClick={() => void retryStranded()}
              >
                <RotateCounterClockwiseIcon />
                {followupRecoveryActionLabel(recovery, t)}
              </button>
            ) : null}
            {revocable ? (
              <button
                type="button"
                className="icon-button"
                disabled={busy}
                onClick={() => onRevoke(message.id)}
                aria-label={t("chat.revokeTitle")}
                title={t("chat.revokeTitle")}
              >
                <Cross2Icon />
              </button>
            ) : null}
          </div>
        ) : null}
      </div>
      <AttachmentRow attachments={attachments} loadAttachment={loadAttachment} />
      {message.reason ? (
        <p className="queued-message__reason">{message.reason}</p>
      ) : null}
      {recoveryNotice ? (
        <p
          className="queued-message__recovery"
          data-tone={recoveryNotice.tone}
          role={recoveryNotice.tone === "error" ? "alert" : "status"}
        >
          {recoveryNotice.text}
        </p>
      ) : null}
    </article>
  );
}

function messageStatusLabel(
  message: ProductMessage,
  t: (path: string) => string,
): string {
  switch (message.status) {
    case "queued":
      // The ledger row says where the message is headed, not just that it waits.
      return `${t("inspector.statusQueued")} · ${deliveryLabel(message.requested_delivery, t)}`;
    case "intervention_requested":
      return t("chat.statusJoiningTurn");
    case "applied_current_run":
      return t("chat.statusJoinedTurn");
    case "claimed_successor":
      return t("chat.statusNextTurn");
    case "needs_attention":
      return t("workspace.needsAttention");
    case "revoked":
      return t("chat.revoke");
  }
}

function deliveryLabel(
  delivery: ProductMessage["requested_delivery"],
  t: (path: string) => string,
): string {
  return delivery === "current_run"
    ? t("chat.deliveryThisTurn")
    : t("chat.deliveryNextTurn");
}
