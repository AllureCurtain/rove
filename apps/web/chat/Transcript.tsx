"use client";

import {
  CheckIcon,
  ArrowDownIcon,
  ArrowUpIcon,
  ChevronDownIcon,
  CodeIcon,
  CopyIcon,
  Cross2Icon,
  FileIcon,
  FilePlusIcon,
  LockClosedIcon,
  Pencil1Icon,
  RotateCounterClockwiseIcon,
} from "@radix-ui/react-icons";
import { FormEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";

import type {
  ChatMessage,
  ToolCallView,
  TranscriptInputView,
  TranscriptRunGroup,
} from "../lib/rove-state";
import { useCopy } from "../copy/CopyProvider";
import { DiffView } from "../product-v2/DiffView";
import { RichText } from "../product-v2/RichText";
import type { ProductMessage } from "../product/product-api-types";
import { ConversationMinimap } from "./ConversationMinimap";
import { ReadingWidthHandles } from "./ReadingWidthHandles";
import { buildConversationMinimapMarkers } from "./conversation-minimap";
import { buildToolPresentation, type ToolBlock } from "./tool-presentation";
import {
  activityWantsOpen,
  buildTranscriptEntries,
  type TranscriptEntry,
} from "./transcript-entries";
import {
  initialDisclosure,
  reduceDisclosure,
  type DisclosureState,
} from "./disclosure";
import {
  INITIAL_MOUNTED,
  MOUNT_STEP,
  initialTranscriptWindow,
  reduceTranscriptWindow,
} from "./transcript-window";
import { useFollowScroll } from "./use-follow-scroll";
import {
  describeTranscriptPartialReason,
  type TranscriptRestoreState,
} from "../state/transcript-projection";

/** Sub-pixel slack before the transcript counts as scrolling. */
const TRANSCRIPT_OVERFLOW_SLACK_PX = 4;

export function Transcript({
  timeline,
  messages = [],
  messageBusy = null,
  canPromote = false,
  approvalBusy,
  inputBusy,
  restoreState,
  onRetryRestore,
  onStartNewSession,
  onApproval,
  onApprovalDetail,
  approvalError,
  onInputSubmit,
  onPromoteMessage = () => {},
  onRevokeMessage = () => {},
  onEditQueuedMessage,
  onMoveQueuedMessage,
  editingQueuedMessageId = null,
  onRetryMessage,
  onEditMessage,
  onForkSession,
  forkAvailable = false,
}: {
  timeline: TranscriptRunGroup[];
  messages?: ProductMessage[];
  messageBusy?: string | null;
  canPromote?: boolean;
  approvalBusy: string | null;
  inputBusy: string | null;
  restoreState: TranscriptRestoreState;
  onRetryRestore: () => void;
  onStartNewSession: () => void;
  onApproval: (tool: ToolCallView, decision: "approve" | "reject") => void;
  onApprovalDetail?: (tool: ToolCallView, trigger: HTMLElement) => void;
  approvalError?: string | null;
  onInputSubmit: (inputId: string, answer: string) => void;
  onPromoteMessage?: (messageId: string) => void;
  onRevokeMessage?: (messageId: string) => void;
  /** Load a queued message back into the composer for replacement. */
  onEditQueuedMessage?: (messageId: string, content: string) => void;
  /** Adjacent swap within the successor queue. */
  onMoveQueuedMessage?: (messageId: string, direction: "up" | "down") => void;
  /** The queued message currently loaded for editing, if any. */
  editingQueuedMessageId?: string | null;
  /** Resend the prompt of the last turn as a new message (no truncation). */
  onRetryMessage?: (content: string) => void;
  /** Load the last user message into the composer (edit-resend, degraded). */
  onEditMessage?: (content: string) => void;
  /** Session-level fork surfaced from message actions. */
  onForkSession?: () => void;
  forkAvailable?: boolean;
}) {
  const { t } = useCopy();
  // Two layers stay apart: how many runs the projection holds versus how many
  // are on the DOM. Growing mounts in-memory runs first; only a drained mount
  // window proposes a server page (no transcript cursor exists yet).
  const [transcriptWindow, setTranscriptWindow] = useState(() =>
    initialTranscriptWindow(timeline.length),
  );
  useEffect(() => {
    setTranscriptWindow((current) => {
      if (timeline.length < current.loaded) {
        // A session switch replaced the timeline: start the window fresh.
        return initialTranscriptWindow(timeline.length);
      }
      if (timeline.length === current.loaded) {
        return current;
      }
      const grown = reduceTranscriptWindow(current, {
        type: "loaded",
        count: timeline.length - current.loaded,
      });
      // Content arriving on a fresh or restored transcript opens at least the
      // initial slice, and a live tail run is always inside the last slice —
      // the window is tail-anchored, so nothing the user is watching needs a
      // click to appear. Growing beyond the initial slice stays a user action.
      return {
        ...grown,
        mounted: Math.max(grown.mounted, Math.min(INITIAL_MOUNTED, timeline.length)),
      };
    });
  }, [timeline.length]);
  const visibleTimeline = useMemo(
    () => timeline.slice(Math.max(0, timeline.length - transcriptWindow.mounted)),
    [timeline, transcriptWindow.mounted],
  );
  // Disclosure state lives here rather than inside the cards: windowing or a
  // session switch can unmount a group, and the reader's takeover must survive
  // that until the entry's identity is gone.
  const [disclosure, setDisclosure] = useState<Map<string, DisclosureState>>(
    () => new Map(),
  );
  const visibleEntries = useMemo(
    () =>
      visibleTimeline.map((group) => ({
        group,
        entries: buildTranscriptEntries(group.items),
      })),
    [visibleTimeline],
  );
  const actionableMessages = useMemo(
    () => messages.filter((message) =>
      message.status === "queued" ||
      message.status === "intervention_requested" ||
      message.status === "needs_attention"
    ),
    [messages],
  );
  const hiddenRunCount = Math.max(0, timeline.length - visibleTimeline.length);
  // The successor queue in ledger order: adjacency for move up/down.
  const movableQueueIds = useMemo(
    () =>
      messages
        .filter(
          (message) =>
            message.status === "queued" && message.requested_delivery === "successor",
        )
        .sort((left, right) => left.seq - right.seq)
        .map((message) => message.id),
    [messages],
  );
  // Which answer gets retry, and which turn gets edit: only the final answer
  // and the final user turn offer the actions (degraded edit-resend).
  const lastActionable = useMemo(() => {
    let answerId: string | null = null;
    let retryContent: string | null = null;
    let turnId: string | null = null;
    let turnContent: string | null = null;
    for (const { entries } of visibleEntries) {
      for (const entry of entries) {
        if (entry.kind === "turn") {
          turnId = entry.id;
          turnContent = entry.message.content;
        } else if (entry.kind === "answer") {
          answerId = entry.id;
          retryContent = turnContent;
        }
      }
    }
    return { answerId, retryContent, turnId };
  }, [visibleEntries]);
  const itemCount = visibleTimeline.reduce((total, group) => total + group.items.length, 0)
    + actionableMessages.length;
  // One marker per rendered turn: jumping is only offered to what is on screen.
  const minimapMarkers = useMemo(
    () =>
      buildConversationMinimapMarkers(
        visibleTimeline.flatMap((group) =>
          group.items.flatMap((item) =>
            item.kind === "message"
              ? [
                  {
                    id: item.message.id,
                    role: item.message.role,
                    content: item.message.content,
                  },
                ]
              : [],
          ),
        ),
      ),
    [visibleTimeline],
  );
  const {
    scrollRef: transcriptRef,
    showJump,
    handleScroll,
    jumpToLatest,
    pinToLatest,
    followNow,
    recordScrollPosition,
    jumpToMarker,
  } = useFollowScroll();
  const prependHeightRef = useRef<number | null>(null);
  const anchorRef = useRef<{ element: HTMLElement; offset: number } | null>(null);
  const holdAnchor = useCallback(
    (element: HTMLElement) => {
      const transcript = transcriptRef.current;
      // While pinned to the latest turn a height change should land at the new
      // bottom; anchoring would fight the follow state machine.
      if (!transcript || !showJump) {
        return;
      }
      anchorRef.current = {
        element,
        offset: element.getBoundingClientRect().top - transcript.getBoundingClientRect().top,
      };
    },
    [showJump, transcriptRef],
  );
  const toggleActivity = useCallback(
    (activityId: string, head: HTMLElement, open: boolean) => {
      holdAnchor(head);
      setDisclosure((current) => {
        const state = current.get(activityId) ?? initialDisclosure(open);
        const map = new Map(current);
        map.set(activityId, reduceDisclosure(state, { type: "user", open: !open }));
        return map;
      });
    },
    [holdAnchor],
  );
  // Adopt new activity ids from the run state and drop ids that vanished with
  // a replaced run. Automatic updates stop once the reader took over.
  useEffect(() => {
    const liveIds = new Set<string>();
    for (const { entries } of visibleEntries) {
      for (const entry of entries) {
        if (entry.kind === "activity") {
          liveIds.add(entry.id);
        }
      }
    }
    setDisclosure((current) => {
      let next: Map<string, DisclosureState> | null = null;
      for (const id of current.keys()) {
        if (!liveIds.has(id)) {
          next = next ?? new Map(current);
          next.delete(id);
        }
      }
      for (const { entries } of visibleEntries) {
        for (const entry of entries) {
          if (entry.kind !== "activity") {
            continue;
          }
          const wantOpen = activityWantsOpen(entry);
          const state = (next ?? current).get(entry.id);
          if (!state) {
            next = next ?? new Map(current);
            next.set(entry.id, initialDisclosure(wantOpen));
            continue;
          }
          const reduced = reduceDisclosure(state, { type: "auto", wantOpen });
          // reduceDisclosure always allocates a fresh state for auto events;
          // compare by value or the effect feeds itself forever.
          if (
            reduced.open !== state.open ||
            reduced.userControlled !== state.userControlled
          ) {
            next = next ?? new Map(current);
            next.set(entry.id, reduced);
          }
        }
      }
      return next ?? current;
    });
  }, [visibleEntries]);
  const [transcriptOverflows, setTranscriptOverflows] = useState(false);

  useEffect(() => {
    const transcript = transcriptRef.current;
    if (!transcript) {
      return;
    }
    const observer = new ResizeObserver(() => {
      if (anchorRef.current !== null) {
        // A disclosure toggle changed the height: keep the toggled title at
        // the viewport offset it had, so the reader does not drift.
        const anchor = anchorRef.current;
        anchorRef.current = null;
        if (anchor.element.isConnected) {
          const delta =
            anchor.element.getBoundingClientRect().top -
            transcript.getBoundingClientRect().top -
            anchor.offset;
          transcript.scrollTop += delta;
          recordScrollPosition(transcript.scrollTop);
          return;
        }
      }
      if (prependHeightRef.current !== null) {
        // Loading older turns pushes everything down: keep the reading position
        // and tell the follow state machine where the scroller landed, so the
        // correction is not mistaken for the user scrolling up.
        transcript.scrollTop += transcript.scrollHeight - prependHeightRef.current;
        prependHeightRef.current = null;
        recordScrollPosition(transcript.scrollTop);
        return;
      }
      // The minimap only earns its lane once the transcript actually scrolls.
      // React bails out when the boolean is unchanged, so repeated observer
      // callbacks at a settled size cost nothing.
      setTranscriptOverflows(
        transcript.scrollHeight - transcript.clientHeight > TRANSCRIPT_OVERFLOW_SLACK_PX,
      );
      // Re-pin from the observer callback itself: a frame scheduled from here
      // paints one unpinned frame before the follow lands.
      followNow();
    });
    const content = transcript.firstElementChild ?? transcript;
    observer.observe(content);
    // Observe the scroller as well as its content. A taller composer (or a new
    // queued-message row) shrinks the viewport by tens of pixels without
    // touching the content box, which leaves the pinned position short of the
    // bottom — measured 22px at 1280x800 — until something else re-pins.
    observer.observe(transcript);
    followNow();
    return () => observer.disconnect();
  }, [followNow, recordScrollPosition, transcriptRef]);

  useEffect(() => {
    // A session switch starts at its newest turn; the window itself is reset
    // by the timeline-length effect when the shorter timeline arrives.
    pinToLatest();
  }, [pinToLatest, restoreState.status === "idle" ? "idle" : restoreState.sessionId]);

  function loadOlderRuns() {
    const transcript = transcriptRef.current;
    if (transcript) {
      prependHeightRef.current = transcript.scrollHeight;
    }
    setTranscriptWindow((current) => reduceTranscriptWindow(current, { type: "grow" }));
  }

  return (
    <div className="chat-transcript-frame">
      {approvalError ? <p className="shell-alert" role="alert">{approvalError}</p> : null}
      <div
        ref={transcriptRef}
        className="chat-transcript"
        aria-label="Conversation"
        role="log"
        aria-live="polite"
        aria-relevant="additions text"
        onScroll={handleScroll}
      >
        <div className="chat-transcript__content">
          <RestoreNotice
            state={restoreState}
            onRetry={onRetryRestore}
            onStartNewSession={onStartNewSession}
          />
          {hiddenRunCount > 0 ? (
            <button type="button" className="load-older-turns" onClick={loadOlderRuns}>
              {t("chat.loadOlder", { n: Math.min(MOUNT_STEP, hiddenRunCount) })}
            </button>
          ) : null}
          {itemCount === 0 &&
          (restoreState.status === "complete" || restoreState.status === "idle") ? (
            <p className="transcript-empty">{t("chat.emptyTranscript")}</p>
          ) : null}
          {visibleEntries.map(({ group, entries }) => (
          <section
            key={group.id}
            className="transcript-run"
            data-run-id={group.runId ?? undefined}
            data-run-ordinal={group.runOrdinal ?? undefined}
            data-inherited={group.inherited ? "true" : undefined}
          >
            <header className="transcript-run__header">
              <span className="transcript-run__label">
                <span>
                  {group.runOrdinal
                    ? t("chat.turn", { n: group.runOrdinal })
                    : t("chat.currentTurn")}
                </span>
                {group.inherited ? (
                  <small>{t("workspace.forked")}</small>
                ) : null}
              </span>
            </header>
            {entries.map((entry) =>
              entry.kind === "activity" ? (
                <ActivityGroup
                  key={entry.id}
                  entry={entry}
                  open={disclosure.get(entry.id)?.open ?? activityWantsOpen(entry)}
                  onToggle={toggleActivity}
                  approvalBusy={approvalBusy}
                  inputBusy={inputBusy}
                  onApproval={onApproval}
                  onApprovalDetail={onApprovalDetail}
                  onInputSubmit={onInputSubmit}
                />
              ) : (
                <MessageBubble
                  key={entry.id}
                  message={entry.message}
                  retryContent={
                    entry.id === lastActionable.answerId
                      ? lastActionable.retryContent
                      : null
                  }
                  onRetry={onRetryMessage}
                  onEdit={
                    entry.id === lastActionable.turnId ? onEditMessage : undefined
                  }
                  onFork={forkAvailable && entry.message.role === "user" ? onForkSession : undefined}
                />
              ),
            )}
          </section>
          ))}
          {actionableMessages.map((message) => {
            const queueIndex = movableQueueIds.indexOf(message.id);
            return (
              <QueuedMessage
                key={message.id}
                message={message}
                busy={messageBusy !== null}
                canPromote={canPromote}
                editing={message.id === editingQueuedMessageId}
                movable={{
                  up: queueIndex > 0,
                  down: queueIndex >= 0 && queueIndex < movableQueueIds.length - 1,
                }}
                onPromote={onPromoteMessage}
                onRevoke={onRevokeMessage}
                onEdit={onEditQueuedMessage}
                onMove={onMoveQueuedMessage}
              />
            );
          })}
        </div>
      </div>
      {showJump && itemCount > 0 ? (
        <button type="button" className="return-to-latest" onClick={jumpToLatest}>
          {t("chat.returnToLatest")}
        </button>
      ) : null}
      <ConversationMinimap
        markers={minimapMarkers}
        overflows={transcriptOverflows}
        hasEarlier={hiddenRunCount > 0}
        onJumpTo={jumpToMarker}
      />
      {/* Dual edge handles for the centered reading band (PI-Desktop D439). */}
      <ReadingWidthHandles />
    </div>
  );
}

function QueuedMessage({
  message,
  busy,
  canPromote,
  editing,
  movable,
  onPromote,
  onRevoke,
  onEdit,
  onMove,
}: {
  message: ProductMessage;
  busy: boolean;
  canPromote: boolean;
  /** The message is loaded into the composer for replacement: lock other ops. */
  editing: boolean;
  movable: { up: boolean; down: boolean };
  onPromote: (messageId: string) => void;
  onRevoke: (messageId: string) => void;
  onEdit?: (messageId: string, content: string) => void;
  onMove?: (messageId: string, direction: "up" | "down") => void;
}) {
  const { t } = useCopy();
  const promotable = message.status === "queued" && canPromote;
  const revocable = message.status === "queued" || message.status === "needs_attention";
  // Only queued successor messages take part in edit and adjacent swap; an
  // intervention targets the running turn and its order is not negotiable.
  const editable =
    message.status === "queued" &&
    message.requested_delivery === "successor" &&
    !editing;
  return (
    <article
      className="chat-bubble queued-message"
      data-role="user"
      data-status={message.status}
      data-delivery={message.requested_delivery}
      data-editing={editing || undefined}
    >
      <div className="message-byline">
        <strong>{t("chat.you")}</strong>
        <span>{messageStatusLabel(message, t)}</span>
      </div>
      <RichText content={message.content} />
      {message.reason ? <p className="queued-message__reason">{message.reason}</p> : null}
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
            <button type="button" className="secondary" disabled={busy} onClick={() => onPromote(message.id)}>
              <ArrowUpIcon />
              {t("chat.promote")}
            </button>
          ) : null}
          {revocable ? (
            <button type="button" className="icon-button" disabled={busy} onClick={() => onRevoke(message.id)} aria-label={t("chat.revokeTitle")} title={t("chat.revokeTitle")}>
              <Cross2Icon />
            </button>
          ) : null}
        </div>
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

function MessageBubble({
  message,
  retryContent = null,
  onRetry,
  onEdit,
  onFork,
}: {
  message: ChatMessage;
  /** The prompt to resend; only the final answer carries it. */
  retryContent?: string | null;
  onRetry?: (content: string) => void;
  onEdit?: (content: string) => void;
  onFork?: () => void;
}) {
  const { t } = useCopy();
  return (
    <div className="transcript-item" data-kind="message">
      <div className="transcript-item__meta" aria-hidden="true">
        <span data-state={message.status} />
      </div>
      <div className="transcript-item__content">
        <article
          className="chat-bubble"
          data-role={message.role}
          data-status={message.status}
          // Anchor for the conversation minimap: one marker per turn message.
          data-message-id={message.id}
        >
          <div className="message-byline">
            <strong>
              {message.role === "user" ? t("chat.you") : t("chat.assistant")}
            </strong>
            <span>
              {message.status === "streaming"
                ? t("chat.responding")
                : ""}
            </span>
          </div>
          <RichText content={message.content} />
          <MessageActions
            content={message.content}
            retryContent={message.role === "assistant" ? retryContent : null}
            onRetry={onRetry}
            onEdit={message.role === "user" ? onEdit : undefined}
            onFork={message.role === "user" ? onFork : undefined}
          />
          {message.role === "assistant" ? (
            <MessageEvidence message={message} />
          ) : null}
        </article>
      </div>
    </div>
  );
}

/**
 * One folded run of tool and input work between two messages.
 *
 * Blocking interactions — a pending approval, a waiting input — never live
 * inside the collapsible body: the reader took over the fold, but the run is
 * still parked on that interaction and it has to stay reachable.
 */
function ActivityGroup({
  entry,
  open,
  onToggle,
  approvalBusy,
  inputBusy,
  onApproval,
  onApprovalDetail,
  onInputSubmit,
}: {
  entry: Extract<TranscriptEntry, { kind: "activity" }>;
  open: boolean;
  onToggle: (activityId: string, head: HTMLElement, open: boolean) => void;
  approvalBusy: string | null;
  inputBusy: string | null;
  onApproval: (tool: ToolCallView, decision: "approve" | "reject") => void;
  onApprovalDetail?: (tool: ToolCallView, trigger: HTMLElement) => void;
  onInputSubmit: (inputId: string, answer: string) => void;
}) {
  const { t } = useCopy();
  const inFlight = activityWantsOpen(entry);
  const isBlockingTool = (tool: ToolCallView) =>
    tool.status === "waiting" || Boolean(tool.pendingApproval);
  const blockingTools = entry.tools.filter(isBlockingTool);
  const bodyTools = entry.tools.filter((tool) => !isBlockingTool(tool));
  const waitingInputs = entry.inputs.filter((input) => input.status === "waiting");
  const settledInputs = entry.inputs.filter((input) => input.status !== "waiting");
  const foldable = bodyTools.length + settledInputs.length > 0;
  const bodyId = `activity-body-${entry.id}`.replace(/[^A-Za-z0-9_-]/gu, "-");

  const renderTool = (tool: ToolCallView) =>
    isBlockingTool(tool) ? (
      <ApprovalCard
        key={tool.timelineId ?? tool.id}
        tool={tool}
        busy={approvalBusy === tool.id}
        onApproval={onApproval}
        onDetail={onApprovalDetail}
      />
    ) : (
      <ToolCard key={tool.timelineId ?? tool.id} tool={tool} />
    );

  const renderInput = (input: TranscriptInputView) =>
    input.status === "waiting" ? (
      <InputCard
        key={input.timelineId}
        inputId={input.id}
        prompt={input.prompt}
        busy={inputBusy === input.id}
        onSubmit={onInputSubmit}
      />
    ) : (
      <article
        className="input-card"
        data-status={input.status}
        role="status"
        key={input.timelineId}
      >
        <div>
          <strong>
            {input.status === "submitted"
              ? t("chat.inputSubmitted")
              : t("chat.inputClosed")}
          </strong>
          <p>{input.prompt}</p>
        </div>
      </article>
    );

  return (
    <div className="transcript-item" data-kind="activity">
      <div className="transcript-item__meta" aria-hidden="true">
        <span data-state={inFlight ? "running" : "done"} />
      </div>
      <div className="transcript-item__content">
        <div
          className="activity-group"
          data-open={open && foldable ? "true" : undefined}
        >
          {blockingTools.map(renderTool)}
          {waitingInputs.map(renderInput)}
          {foldable ? (
            <>
              <button
                type="button"
                className="activity-group__head"
                aria-controls={bodyId}
                aria-expanded={open}
                onClick={(event) => onToggle(entry.id, event.currentTarget, open)}
              >
                <CodeIcon />
                <strong>
                  {t("chat.activitySummary", { n: bodyTools.length })}
                </strong>
                {entry.tools.length + entry.inputs.length >
                bodyTools.length + settledInputs.length ? (
                  <span className="activity-group__hint">
                    {t("chat.activityBlocking", {
                      n:
                        entry.tools.length +
                        entry.inputs.length -
                        bodyTools.length -
                        settledInputs.length,
                    })}
                  </span>
                ) : null}
                <ChevronDownIcon data-open={open} />
              </button>
              {open ? (
                <div className="activity-group__body" id={bodyId}>
                  {bodyTools.map(renderTool)}
                  {settledInputs.map(renderInput)}
                </div>
              ) : null}
            </>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function RestoreNotice({
  state,
  onRetry,
  onStartNewSession,
}: {
  state: TranscriptRestoreState;
  onRetry: () => void;
  onStartNewSession: () => void;
}) {
  const { t } = useCopy();
  if (state.status === "idle" || state.status === "complete") {
    return null;
  }
  if (state.status === "loading") {
    return (
      <section className="restore-notice" data-tone="loading" role="status">
        <strong>{t("chat.restoring")}</strong>
      </section>
    );
  }
  if (state.status === "partial") {
    return (
      <section className="restore-notice" data-tone="partial" role="status">
        <strong>{t("chat.restorePartialTitle")}</strong>
        <span>{t("chat.restorePartial")}</span>
        <ul>
          {state.reasons.map((reason, index) => (
            <li key={`${reason.code}-${reason.run_ordinal ?? "session"}-${index}`}>
              {describeTranscriptPartialReason(reason)}
            </li>
          ))}
        </ul>
        <div className="field-actions">
          <button type="button" className="secondary" onClick={onRetry}>
            {t("chat.restoreRetry")}
          </button>
          <button type="button" className="secondary" onClick={onStartNewSession}>
            {t("nav.newSession")}
          </button>
        </div>
      </section>
    );
  }
  return (
    <section className="restore-notice" data-tone="error" role="alert">
      <strong>{t("chat.restoreFailed")}</strong>
      <span>{state.error}</span>
      <span>{t("chat.restoreError")}</span>
      <div className="field-actions">
        <button type="button" onClick={onRetry}>
          {t("chat.restoreRetry")}
        </button>
        <button type="button" className="secondary" onClick={onStartNewSession}>
          {t("nav.newSession")}
        </button>
      </div>
    </section>
  );
}

function ToolCard({ tool }: { tool: ToolCallView }) {
  const { t } = useCopy();
  const [open, setOpen] = useState(
    tool.status === "running" || tool.status === "error" || Boolean(tool.mutations?.length),
  );
  const detailId = `tool-detail-${tool.timelineId ?? tool.id}`.replace(/[^A-Za-z0-9_-]/gu, "-");
  // Body blocks are built only while open, so a collapsed card stays cheap.
  const presentation = buildToolPresentation(tool, open);
  return (
    <article className="tool-card" data-status={tool.status}>
      <button
        type="button"
        className="tool-card__head"
        aria-controls={detailId}
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        <CodeIcon />
        <span>
          <strong>{presentation.title}</strong>
          {presentation.subtitle ? <small>{presentation.subtitle}</small> : null}
        </span>
        {presentation.chips.map((chip) => (
          <span className="tool-card__chip" data-tone={chip.tone} key={chip.label}>
            {toolChipLabel(chip.label, t)}
          </span>
        ))}
        <ChevronDownIcon data-open={open} />
      </button>
      {open ? (
        <div className="tool-card__details" id={detailId}>
          {presentation.blocks.map((block, index) => (
            <ToolBlockView block={block} key={index} />
          ))}
        </div>
      ) : null}
    </article>
  );
}

/**
 * The presentation model uses stable label vocabulary for chips; only the
 * fixed vocabulary is translated here. Dynamic values (tool status, error
 * codes) are data, not copy, and render as-is.
 */
function toolChipLabel(label: string, t: (path: string) => string): string {
  switch (label) {
    case "read only":
      return t("chat.chipReadOnly");
    case "mutation capable":
      return t("chat.chipMutation");
    case "high risk":
      return t("chat.chipHighRisk");
    case "workspace changed":
      return t("chat.chipWorkspaceChanged");
    default:
      return label;
  }
}

function ToolBlockView({ block }: { block: ToolBlock }) {
  const { t } = useCopy();
  switch (block.kind) {
    case "diff":
      return (
        <section>
          <h4>{block.path}</h4>
          <DiffView diff={block.text} label={`${block.path} diff`} sourcePath={block.path} />
          {block.truncated ? <p className="tool-card__truncated">{t("chat.diffTruncated")}</p> : null}
        </section>
      );
    case "code":
      return (
        <section>
          <RichText content={"```" + block.language + "\n" + block.text + "\n```"} />
          {block.truncated ? <p className="tool-card__truncated">{t("chat.outputTruncated")}</p> : null}
        </section>
      );
    case "files":
      return (
        <section>
          <h4>{t("chat.filesTitle")}</h4>
          <ul className="tool-card__list">
            {block.entries.map((entry) => (
              <li key={entry.path}>{entry.path}</li>
            ))}
          </ul>
        </section>
      );
    case "matches":
      return (
        <section>
          <h4>{t("chat.matchesTitle")}</h4>
          <ul className="tool-card__list">
            {block.entries.map((entry, index) => (
              <li key={index}>
                <code>{entry.path}{entry.line ? ":" + entry.line : ""}</code> {entry.text}
              </li>
            ))}
          </ul>
          {block.truncated ? <p className="tool-card__truncated">{t("chat.matchesTruncated")}</p> : null}
        </section>
      );
    case "fields":
      return (
        <section>
          <dl className="tool-facts">
            {block.rows.map((row, index) => (
              <div key={index}><dt>{row.label}</dt><dd>{row.value}</dd></div>
            ))}
          </dl>
        </section>
      );
    case "note":
      return <pre tabIndex={0}>{block.text}</pre>;
  }
}

function MessageActions({
  content,
  retryContent = null,
  onRetry,
  onEdit,
  onFork,
}: {
  content: string;
  /** Prompt to resend; present only on the final assistant reply. */
  retryContent?: string | null;
  onRetry?: (content: string) => void;
  onEdit?: (content: string) => void;
  onFork?: () => void;
}) {
  const { t } = useCopy();
  const [copied, setCopied] = useState(false);
  const resetRef = useRef<number | null>(null);

  useEffect(() => () => {
    if (resetRef.current !== null) window.clearTimeout(resetRef.current);
  }, []);

  if (!content) return null;

  async function copy() {
    try {
      await navigator.clipboard.writeText(content);
      setCopied(true);
      if (resetRef.current !== null) window.clearTimeout(resetRef.current);
      resetRef.current = window.setTimeout(() => {
        setCopied(false);
        resetRef.current = null;
      }, 1_500);
    } catch {
      setCopied(false);
    }
  }

  return (
    <div className="message-actions">
      {onRetry && retryContent ? (
        <button
          type="button"
          className="ghost icon-button"
          onClick={() => onRetry(retryContent)}
          aria-label={t("chat.retry")}
          title={t("chat.retry")}
        >
          <RotateCounterClockwiseIcon />
        </button>
      ) : null}
      {onEdit ? (
        <button
          type="button"
          className="ghost icon-button"
          onClick={() => onEdit(content)}
          aria-label={t("chat.editMessage")}
          title={t("chat.editMessage")}
        >
          <Pencil1Icon />
        </button>
      ) : null}
      {onFork ? (
        <button
          type="button"
          className="ghost icon-button"
          onClick={onFork}
          aria-label={t("chat.forkSession")}
          title={t("chat.forkSession")}
        >
          <FilePlusIcon />
        </button>
      ) : null}
      <button
        type="button"
        className="ghost icon-button"
        onClick={() => void copy()}
        aria-label={copied ? t("chat.messageCopied") : t("chat.messageCopy")}
        title={copied ? t("chat.messageCopied") : t("chat.messageCopy")}
      >
        {copied ? <CheckIcon /> : <CopyIcon />}
      </button>
    </div>
  );
}

function ToolFacts({ tool }: { tool: ToolCallView }) {
  const metadata = tool.metadata!;
  const affectedPaths = metadata.affected_paths ?? [];
  const diffSummary = metadata.diff_summary ?? [];
  return (
    <section>
      <h4>Execution facts</h4>
      <dl className="tool-facts">
        <div><dt>Call</dt><dd><code>{tool.id}</code></dd></div>
        <div><dt>Status</dt><dd>{metadata.status}</dd></div>
        <div><dt>Risk</dt><dd>{metadata.risk_level}</dd></div>
        <div><dt>Access</dt><dd>{metadata.read_only ? "Read only" : "Mutation capable"}</dd></div>
        <div><dt>Workspace</dt><dd>{metadata.workspace_changed ? "Changed" : "Unchanged"}</dd></div>
        {metadata.error_code ? <div><dt>Error code</dt><dd>{metadata.error_code}</dd></div> : null}
        {metadata.security_event_type ? <div><dt>Security event</dt><dd>{metadata.security_event_type}</dd></div> : null}
        {affectedPaths.length ? (
          <div><dt>Affected paths</dt><dd>{affectedPaths.join(", ")}</dd></div>
        ) : null}
      </dl>
      {diffSummary.length ? (
        <ul className="tool-diff-summary">
          {diffSummary.map((summary, index) => <li key={`${summary}-${index}`}>{summary}</li>)}
        </ul>
      ) : null}
    </section>
  );
}

export function ApprovalCard({
  tool,
  busy,
  onApproval,
  onDetail,
}: {
  tool: ToolCallView;
  busy: boolean;
  onApproval: (tool: ToolCallView, decision: "approve" | "reject") => void;
  onDetail?: (tool: ToolCallView, trigger: HTMLElement) => void;
}) {
  const { t } = useCopy();

  return (
    <article
      className="approval-card"
      aria-label={t("inspector.approvalsTitle")}
      role="alert"
      tabIndex={-1}
    >
      <div className="approval-card__head">
        <LockClosedIcon />
        <span><strong>{t("inspector.statusWaitingApproval")}</strong><small>{tool.name}</small></span>
      </div>
      <p>{tool.reason ?? tool.details}</p>
      <p>{t("inspector.approvalScope")}</p>
      {busy ? <p role="status">{t("inspector.approvalSubmitting")}</p> : null}
      {onDetail ? <button type="button" className="secondary"
        onClick={(event) => onDetail(tool, event.currentTarget)}>
        {t("inspector.approvalDetail")}
      </button> : null}
      {tool.args !== undefined || tool.pendingApproval ? (
        <pre tabIndex={0}>{formatValue(tool.args ?? tool.pendingApproval?.args)}</pre>
      ) : null}
      <div className="field-actions">
        <button
          type="button"
          disabled={busy || !tool.pendingApproval}
          onClick={() => onApproval(tool, "approve")}
        >
          <CheckIcon />
          {t("chat.approve")}
        </button>
        <button
          type="button"
          className="danger"
          disabled={busy || !tool.pendingApproval}
          onClick={() => onApproval(tool, "reject")}
        >
          <Cross2Icon />
          {t("chat.reject")}
        </button>
      </div>
    </article>
  );
}

function InputCard({
  inputId,
  prompt,
  busy,
  onSubmit,
}: {
  inputId: string;
  prompt: string;
  busy: boolean;
  onSubmit: (inputId: string, answer: string) => void;
}) {
  const { t } = useCopy();
  const [answer, setAnswer] = useState("");
  const answerRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    answerRef.current?.focus();
  }, []);

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    const trimmed = answer.trim();
    if (!trimmed || busy) {
      return;
    }
    onSubmit(inputId, trimmed);
  }

  return (
    <article className="input-card" role="status" aria-live="polite">
      <div>
        <strong>{t("chat.responding")}</strong>
        <p>{prompt}</p>
      </div>
      <form className="chat-composer__row" onSubmit={handleSubmit}>
        <input
          ref={answerRef}
          type="text"
          value={answer}
          onChange={(event) => setAnswer(event.target.value)}
          placeholder={t("chat.placeholder")}
          disabled={busy}
          aria-label={prompt}
        />
        <button type="submit" disabled={busy || !answer.trim()}>
          {t("chat.send")}
        </button>
      </form>
    </article>
  );
}

function MessageEvidence({ message }: { message: ChatMessage }) {
  const { t } = useCopy();
  if (!message.usage && !message.promptBuild && !message.promptCompaction) {
    return null;
  }
  return (
    <dl className="message-evidence" aria-label={t("inspector.usageEvidence")}>
      {message.usage ? (
        <>
          <div><dt>{t("inspector.total")}</dt><dd>{formatNumber(message.usage.total_tokens)}</dd></div>
          <div><dt>{t("inspector.prompt")}</dt><dd>{formatNumber(message.usage.prompt_tokens)}</dd></div>
          <div><dt>{t("inspector.completion")}</dt><dd>{formatNumber(message.usage.completion_tokens)}</dd></div>
          {message.usage.cached_tokens !== undefined ? (
            <div><dt>{t("inspector.cached")}</dt><dd>{formatNumber(message.usage.cached_tokens)}</dd></div>
          ) : null}
        </>
      ) : null}
      {message.promptBuild ? (
        <div>
          <dt>{t("inspector.contextEstimate")}</dt>
          <dd>{formatNumber(message.promptBuild.token_estimate)}</dd>
        </div>
      ) : null}
      {message.promptCompaction ? (
        <div><dt>{t("inspector.compacted")}</dt><dd>{message.promptCompaction.mode.replaceAll("_", " ")}</dd></div>
      ) : null}
    </dl>
  );
}

function formatNumber(value: number): string {
  return new Intl.NumberFormat("en-US").format(value);
}

function formatValue(value: unknown): string {
  if (typeof value === "string") {
    return value;
  }
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}
