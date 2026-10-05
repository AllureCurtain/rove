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
import { FormEvent, useCallback, useEffect, useLayoutEffect, useMemo, useReducer, useRef, useState } from "react";

import type {
  ChatMessage,
  ToolCallView,
  TranscriptInputView,
  TranscriptRunGroup,
} from "../lib/rove-state";
import { useCopy } from "../copy/CopyProvider";
import { DiffView } from "../product-v2/DiffView";
import { RichText } from "../product-v2/RichText";
import type {
  ProductMessage,
  ProductMessageAttachmentRef,
  ProductMessageDelivery,
} from "../product/product-api-types";
import {
  attachmentsByRunId,
  availabilityCopyKey,
  formatAttachmentBytes,
  isRasterContentType,
} from "./composer-files";
import { actionableQueueOrder, successorQueue } from "./queue-order";
import {
  IDLE_FOLLOWUP_RECOVERY,
  followupRecoveryActionLabel,
  followupRecoveryNotice,
  isStrandedSuccessor,
  reduceFollowupRecovery,
  type FollowupConfirmationResult,
} from "./followup-recovery";
import { formatPruningBytes, promptPruningSummary } from "./prompt-pruning";
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
  NO_OLDER_HISTORY,
  hiddenMountedCount,
  initialTranscriptWindow,
  mountedRunRange,
  mountsRun,
  olderHistoryControl,
  reduceTranscriptWindow,
  type OlderHistoryState,
} from "./transcript-window";
import { anchoredScrollDelta } from "./transcript-scroll";
import { useFollowScroll } from "./use-follow-scroll";
import {
  activityTiming,
  durationBadgeLabel,
  elapsedSeconds,
  inputArrivalKey,
  toolArrivalKey,
  toolArrivals,
} from "./tool-timing";
import {
  captureActiveSessionView,
  registerSessionViewCapture,
  restoreSessionView,
  sessionViewSnapshots,
  transcriptFingerprint,
} from "./session-view-snapshot";
import {
  describeTranscriptPartialReason,
  type TranscriptRestoreState,
} from "../state/transcript-projection";

/** Sub-pixel slack before the transcript counts as scrolling. */
const TRANSCRIPT_OVERFLOW_SLACK_PX = 4;

/**
 * Restoring a viewport waits for the content it was measured against to be laid
 * out — select, then paint — and keeps re-writing until the position sticks:
 * a write into a still-collapsing scroller is silently clamped, which would
 * leave the reader at the top of the transcript.
 */
const SESSION_VIEW_RESTORE_TIMEOUT_MS = 400;

export function Transcript({
  timeline,
  messages = [],
  messageBusy = null,
  canPromote = false,
  approvalBusy,
  inputBusy,
  restoreState,
  olderHistory = NO_OLDER_HISTORY,
  onLoadOlderHistory,
  onRetryRestore,
  onStartNewSession,
  onApproval,
  onApprovalDetail,
  approvalError,
  onInputSubmit,
  onPromoteMessage = () => {},
  onRevokeMessage = () => {},
  onConfirmStrandedSuccessor,
  onEditQueuedMessage,
  onMoveQueuedMessage,
  editingQueuedMessageId = null,
  onRetryMessage,
  onEditMessage,
  onForkSession,
  onBranchMessage,
  branchAvailable = false,
  forkAvailable = false,
  locateRequest = null,
  loadAttachment,
}: {
  timeline: TranscriptRunGroup[];
  messages?: ProductMessage[];
  messageBusy?: string | null;
  canPromote?: boolean;
  approvalBusy: string | null;
  inputBusy: string | null;
  restoreState: TranscriptRestoreState;
  /** Server cursor pages that have not been fetched yet. */
  olderHistory?: OlderHistoryState;
  onLoadOlderHistory?: () => void | Promise<void>;
  onRetryRestore: () => void;
  onStartNewSession: () => void;
  onApproval: (tool: ToolCallView, decision: "approve" | "reject") => void;
  onApprovalDetail?: (tool: ToolCallView, trigger: HTMLElement) => void;
  approvalError?: string | null;
  onInputSubmit: (inputId: string, answer: string) => void;
  onPromoteMessage?: (messageId: string, delivery: ProductMessageDelivery) => void;
  onRevokeMessage?: (messageId: string) => void;
  /**
   * F.5: ask the server to retry one successor it abandoned. Absent hides the
   * recovery action, which is what a shell without the confirm route wants.
   */
  onConfirmStrandedSuccessor?: (
    messageId: string,
  ) => Promise<FollowupConfirmationResult>;
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
  /**
   * R6: edit a user turn and continue in a new session. The callback receives
   * the run the turn belongs to and its text, which is what identifies the
   * turn's ledger message without inventing a second identity.
   */
  onBranchMessage?: (input: { runId: string | null; content: string }) => void;
  branchAvailable?: boolean;
  forkAvailable?: boolean;
  /**
   * R7: reveal one message on request — the transcript half of a search hit.
   * `requestId` makes a repeated reveal of the same message a new request instead
   * of a no-op.
   */
  locateRequest?: { messageId: string; requestId: number } | null;
  /**
   * R8: loads one attachment's bytes through the authenticated transport,
   * already bound to the session by the shell. Absent hides every attachment
   * row, which is what a shell without attachment support wants.
   */
  loadAttachment?: (attachmentId: string) => Promise<Blob>;
}) {
  const { t } = useCopy();
  /**
   * R8: attachment references live on the durable message, not on the run, so
   * the ledger page the transcript already holds is the source. Keying by run
   * id is what lets a run group's user turn show what that turn carried; no
   * second event was added for it (the API deliberately added none).
   */
  const attachmentsByRun = useMemo(() => attachmentsByRunId(messages), [messages]);
  // Two layers stay apart: how many runs the projection holds versus how many
  // the reader has revealed. Growing mounts in-memory runs first; only a drained
  // reveal budget proposes a server page, and only while the server publishes a
  // cursor. The DOM itself holds at most one window of the revealed runs.
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
      // initial slice, and a live tail run is inside the window whenever follow
      // holds the reveal budget at or below the cap (see the `tail` effect
      // below). Growing beyond the initial slice stays a user action.
      return {
        ...grown,
        mounted: Math.max(grown.mounted, Math.min(INITIAL_MOUNTED, timeline.length)),
      };
    });
  }, [timeline.length]);
  const visibleTimeline = useMemo(() => {
    const range = mountedRunRange(transcriptWindow, timeline.length);
    return timeline.slice(range.start, range.end);
  }, [timeline, transcriptWindow.mounted]);
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
  // The ledger arrives in `seq` order, but a reorder rewrites `queue_order` and
  // never `seq`: the pending rows are shown in the order the server will
  // dispatch them, not the order they were created in.
  const actionableMessages = useMemo(
    () => actionableQueueOrder(messages),
    [messages],
  );
  // Runs the DOM is not holding: the loaded runs the reader has not revealed
  // yet, plus whatever the bounded window released from the newest end. The
  // minimap reports it as "there is more above", which is true of both.
  const hiddenRunCount = Math.max(0, timeline.length - visibleTimeline.length);
  // How many runs one growth step reveals, which is not the same number as
  // `hiddenRunCount` once the window is releasing runs from the newest end.
  const unrevealedRunCount = hiddenMountedCount(transcriptWindow);
  const olderControl = olderHistoryControl(transcriptWindow, olderHistory);
  // The loaded run count raised by the last server page. It lets the prepend
  // correction below tell that page's arrival from unrelated growth.
  const olderHistoryRunsRef = useRef<number | null>(null);
  // The successor queue in delivery order (`queue_order ?? seq`): adjacency for
  // move up/down, in the same order the server dispatches it.
  const movableQueueIds = useMemo(
    () => successorQueue(messages).map((message) => message.id),
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
    readFollowState,
    restoreFollowState,
    settleRestoredFollow,
  } = useFollowScroll();
  // Follow means the newest run, and the bounded window can be holding older
  // history instead: once the reader engages follow again — the "return to
  // latest" control, or scrolling back down to the bottom of the window — the
  // window has to include the tail again, or there would be nothing to follow
  // and nothing for a new run to arrive into. `showJump` is false exactly while
  // follow is pinned, and `tail` is a no-op whenever the window already ends at
  // the tail, so this costs nothing on a short transcript.
  useEffect(() => {
    if (showJump) {
      return;
    }
    setTranscriptWindow((current) =>
      reduceTranscriptWindow(current, { type: "tail" }),
    );
  }, [showJump]);
  // R7: reveal a message a search hit named. A reveal is only possible for a run
  // that is on the DOM, so a target that is not on screen re-anchors the window
  // first and the jump happens on the run after that; a target outside the
  // projected runs is left alone, and the overlay keeps reporting what the host
  // could resolve.
  const handledLocateRef = useRef<number | null>(null);
  useLayoutEffect(() => {
    if (locateRequest === null) {
      return;
    }
    if (handledLocateRef.current === locateRequest.requestId) {
      return;
    }
    const index = timeline.findIndex((group) =>
      group.items.some(
        (item) =>
          item.kind === "message" && item.message.id === locateRequest.messageId,
      ),
    );
    if (index === -1) {
      return;
    }
    const needed = timeline.length - index;
    // Not "is it revealed": a run the reader has walked past can be inside the
    // revealed range and still released from the newest end of the window.
    if (!mountsRun(transcriptWindow, needed)) {
      setTranscriptWindow((current) =>
        reduceTranscriptWindow(current, { type: "mount", count: needed }),
      );
      return;
    }
    handledLocateRef.current = locateRequest.requestId;
    jumpToMarker(locateRequest.messageId);
  }, [jumpToMarker, locateRequest, timeline, transcriptWindow.mounted]);
  // The session whose data this transcript currently holds. An empty restore
  // state means there is nothing to snapshot or reinstate.
  const viewSessionId = restoreState.status === "idle" ? null : restoreState.sessionId;
  const currentViewSessionRef = useRef(viewSessionId);
  currentViewSessionRef.current = viewSessionId;
  const timelineFingerprint = useMemo(() => transcriptFingerprint(timeline), [timeline]);
  const fingerprintRef = useRef(timelineFingerprint);
  fingerprintRef.current = timelineFingerprint;
  // The data layer captures right before it resets the transcript, because only
  // it knows a session is about to be replaced. The reader's view is read
  // through a ref so the registered callback never closes over a stale render.
  const captureViewRef = useRef<() => void>(() => {});
  captureViewRef.current = () => {
    // Only a session that actually rendered something has a view worth keeping,
    // and a reset timeline is what a capture after the switch would see.
    if (viewSessionId === null || timeline.length === 0) {
      return;
    }
    const follow = readFollowState();
    sessionViewSnapshots.record(
      viewSessionId,
      {
        scrollTop: follow.scrollTop,
        followPinned: follow.pinned,
        window: transcriptWindow,
        disclosure,
      },
      timelineFingerprint,
    );
  };
  useEffect(() => {
    registerSessionViewCapture(() => captureViewRef.current());
    return () => registerSessionViewCapture(null);
  }, []);
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
  // A growth step reveals older runs above the reader and releases the newest
  // ones below, so `scrollHeight` cannot measure it: its delta is what arrived
  // above minus what left below, and only what arrived above moves the reader.
  // The run at the top of the viewport survives the step, and it is measured.
  const mountedAnchorRef = useRef<{ element: HTMLElement; offset: number } | null>(null);
  const anchorTopRenderedRun = useCallback(() => {
    const transcript = transcriptRef.current;
    if (!transcript) {
      return;
    }
    const top = transcript.getBoundingClientRect().top;
    const run = [
      ...transcript.querySelectorAll<HTMLElement>("[data-run-ordinal]"),
    ].find((section) => section.getBoundingClientRect().top >= top);
    if (!run) {
      return;
    }
    mountedAnchorRef.current = {
      element: run,
      offset: run.getBoundingClientRect().top - top,
    };
  }, [transcriptRef]);
  // Correcting after the commit rather than from the ResizeObserver: a step that
  // reveals as much as it releases can leave the content height unchanged, and
  // the observer would never fire for it.
  useLayoutEffect(() => {
    const anchor = mountedAnchorRef.current;
    if (anchor === null) {
      return;
    }
    mountedAnchorRef.current = null;
    const transcript = transcriptRef.current;
    if (!transcript || !anchor.element.isConnected) {
      return;
    }
    const delta = anchoredScrollDelta({
      anchorTop: anchor.element.getBoundingClientRect().top,
      scrollerTop: transcript.getBoundingClientRect().top,
      anchorOffset: anchor.offset,
    });
    if (delta === 0) {
      return;
    }
    transcript.scrollTop += delta;
    recordScrollPosition(transcript.scrollTop);
  }, [recordScrollPosition, transcriptRef, transcriptWindow.mounted]);

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
        // An older server page changes the control row above the reader without
        // changing which runs are rendered: keep the reading position and tell
        // the follow state machine where the scroller landed, so the correction
        // is not mistaken for the user scrolling up.
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
    // A session switch starts at its newest turn — unless the reader's own view
    // of that session is about to be reinstated by the snapshot effect below.
    if (viewSessionId !== null && sessionViewSnapshots.peek(viewSessionId)) {
      return;
    }
    pinToLatest();
  }, [pinToLatest, viewSessionId]);

  // Reinstating a per-session view happens once the restored data is in place.
  // A snapshot is consumed at most once, and the session is re-checked when the
  // position is finally written, so a capture can never land in the session that
  // replaced it.
  useEffect(() => {
    if (
      restoreState.status === "idle" ||
      restoreState.status === "loading" ||
      restoreState.status === "error"
    ) {
      return;
    }
    const sessionId = restoreState.sessionId;
    const snapshot = sessionViewSnapshots.take(sessionId);
    if (!snapshot) {
      return;
    }
    const restored = restoreSessionView(snapshot, fingerprintRef.current);
    setDisclosure(restored.disclosure);
    if (restored.stale || restored.window === null || restored.scrollTop === null) {
      // The session advanced while it was in the background: its newest turn is
      // the only honest place to land.
      pinToLatest();
      return;
    }
    setTranscriptWindow(restored.window);
    restoreFollowState({
      pinned: restored.followPinned,
      scrollTop: restored.scrollTop,
    });
    const targetTop = restored.scrollTop;
    const startedAt = performance.now();
    let firstFrame = 0;
    let frame = 0;
    const settle = () => {
      const element = transcriptRef.current;
      if (!element || currentViewSessionRef.current !== sessionId) {
        return;
      }
      // `scrollTop =` is animated while the stylesheet asks for smooth scrolling,
      // so the restored position has to be a move rather than a journey.
      element.scrollTo({ top: targetTop, behavior: "instant" });
      // A write that did not land was clamped: the content it was measured
      // against is still arriving. Keep asking until it sticks or the ceiling
      // passes, so the reader's position wins over a late layout.
      const landed = Math.abs(element.scrollTop - targetTop) <= 1;
      if (!landed && performance.now() - startedAt < SESSION_VIEW_RESTORE_TIMEOUT_MS) {
        frame = requestAnimationFrame(settle);
        return;
      }
      settleRestoredFollow();
    };
    // Select-then-paint: the content has to be laid out before a position inside
    // it means anything, and writing into a collapsing scroller is clamped.
    firstFrame = requestAnimationFrame(() => {
      frame = requestAnimationFrame(settle);
    });
    return () => {
      cancelAnimationFrame(firstFrame);
      cancelAnimationFrame(frame);
    };
  }, [
    pinToLatest,
    restoreFollowState,
    restoreState.status,
    settleRestoredFollow,
    transcriptRef,
    viewSessionId,
  ]);

  useLayoutEffect(() => {
    const pendingRuns = olderHistoryRunsRef.current;
    if (pendingRuns === null || timeline.length <= pendingRuns) {
      return;
    }
    // The older page landed. A cursor page never changes the rendered run set —
    // the window is anchored from the tail — but the control row above the
    // reader does change, so correct before paint with the same baseline
    // mechanism the mount-growth path uses.
    olderHistoryRunsRef.current = null;
    const transcript = transcriptRef.current;
    const baseline = prependHeightRef.current;
    prependHeightRef.current = null;
    if (!transcript || baseline === null) {
      return;
    }
    const delta = transcript.scrollHeight - baseline;
    if (delta > 0) {
      transcript.scrollTop += delta;
      recordScrollPosition(transcript.scrollTop);
    }
  }, [recordScrollPosition, timeline.length]);

  useEffect(() => {
    if (olderHistory.loading || olderHistoryRunsRef.current === null) {
      return;
    }
    // No prepend followed the request (a failure, or an empty page): drop the
    // baseline so later growth is never misread as this page arriving.
    olderHistoryRunsRef.current = null;
    prependHeightRef.current = null;
  }, [olderHistory.loading, timeline.length]);

  function loadOlderRuns() {
    // The step both reveals runs above the reader and releases the newest ones,
    // so remember a run that survives it instead of the content height.
    anchorTopRenderedRun();
    setTranscriptWindow((current) => reduceTranscriptWindow(current, { type: "grow" }));
  }

  function loadOlderHistory() {
    const transcript = transcriptRef.current;
    if (transcript) {
      prependHeightRef.current = transcript.scrollHeight;
    }
    olderHistoryRunsRef.current = timeline.length;
    void onLoadOlderHistory?.();
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
          {olderControl === "grow" ? (
            <button type="button" className="load-older-turns" onClick={loadOlderRuns}>
              {t("chat.loadOlder", { n: Math.min(MOUNT_STEP, unrevealedRunCount) })}
            </button>
          ) : null}
          {olderControl === "server" ? (
            <button
              type="button"
              className="load-older-history"
              disabled={olderHistory.loading}
              onClick={loadOlderHistory}
            >
              {olderHistory.loading
                ? t("chat.loadingOlderHistory")
                : t("chat.loadOlderHistory")}
            </button>
          ) : null}
          {olderHistory.error ? (
            <div className="load-older-history__error" role="alert">
              <span>{olderHistory.error}</span>
              <button
                type="button"
                className="secondary"
                disabled={olderHistory.loading}
                onClick={loadOlderHistory}
              >
                {t("chat.retryOlderHistory")}
              </button>
            </div>
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
                  attachments={
                    entry.message.role === "user" && group.runId
                      ? attachmentsByRun.get(group.runId) ?? []
                      : []
                  }
                  loadAttachment={loadAttachment}
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
                  onBranch={
                    branchAvailable &&
                    entry.message.role === "user" &&
                    onBranchMessage
                      ? () =>
                          onBranchMessage({
                            runId: group.runId,
                            content: entry.message.content,
                          })
                      : undefined
                  }
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
                attachments={message.attachments ?? []}
                loadAttachment={loadAttachment}
                busy={messageBusy !== null}
                canPromote={canPromote}
                editing={message.id === editingQueuedMessageId}
                movable={{
                  up: queueIndex > 0,
                  down: queueIndex >= 0 && queueIndex < movableQueueIds.length - 1,
                }}
                onPromote={onPromoteMessage}
                onRevoke={onRevokeMessage}
                onConfirmStranded={onConfirmStrandedSuccessor}
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
      {/* Dual edge handles for the centered reading band. */}
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
      <AttachmentRow attachments={attachments} loadAttachment={loadAttachment} />
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
            // F.5: the successor was abandoned, so the reader's one action is to
            // ask the server to try it again. Disabled while the request is
            // unsettled — the label says which — and the result is the server's,
            // never an assumption.
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
            <button type="button" className="icon-button" disabled={busy} onClick={() => onRevoke(message.id)} aria-label={t("chat.revokeTitle")} title={t("chat.revokeTitle")}>
              <Cross2Icon />
            </button>
          ) : null}
        </div>
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

/**
 * One message's durable attachment references (R8).
 *
 * The row renders the server-verified facts and nothing else, and it loads
 * bytes on request through the authenticated transport rather than linking to
 * them: a bare URL cannot carry the Desktop bearer token. An image expands in
 * place; anything else saves through the same fetch. A reference whose bytes
 * the server reports as gone says so instead of offering a dead fetch, and a
 * reference the server marked as degraded says the model never saw it — the
 * transcript must not imply a model saw an image it did not.
 */
function AttachmentRow({
  attachments,
  loadAttachment,
}: {
  attachments: readonly ProductMessageAttachmentRef[];
  loadAttachment?: (attachmentId: string) => Promise<Blob>;
}) {
  const { t } = useCopy();
  if (attachments.length === 0 || !loadAttachment) {
    return null;
  }
  return (
    <ul className="message-attachments">
      {attachments.map((attachment) => (
        <MessageAttachment
          key={attachment.attachment_id}
          attachment={attachment}
          load={loadAttachment}
          label={t("chat.attachmentLink", {
            name: attachment.name ?? attachment.attachment_id,
            size: formatAttachmentBytes(attachment.size),
          })}
          unavailable={
            availabilityCopyKey(attachment.availability) === null
              ? null
              : t(availabilityCopyKey(attachment.availability) as string)
          }
          degraded={attachment.degradation ? t("chat.attachmentDegraded") : null}
        />
      ))}
    </ul>
  );
}

function MessageAttachment({
  attachment,
  load,
  label,
  unavailable,
  degraded,
}: {
  attachment: ProductMessageAttachmentRef;
  load: (attachmentId: string) => Promise<Blob>;
  label: string;
  unavailable: string | null;
  degraded: string | null;
}) {
  const { t } = useCopy();
  const name = attachment.name ?? attachment.attachment_id;
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const image =
    attachment.availability === "available" &&
    isRasterContentType(attachment.content_type);

  useEffect(
    () => () => {
      if (previewUrl) {
        URL.revokeObjectURL(previewUrl);
      }
    },
    [previewUrl],
  );

  /**
   * The bytes are fetched once, on the reader's request: a transcript page may
   * hold many references, and each one may be as large as the upload cap.
   */
  async function open() {
    if (busy || unavailable) {
      return;
    }
    if (previewUrl) {
      URL.revokeObjectURL(previewUrl);
      setPreviewUrl(null);
      return;
    }
    setBusy(true);
    setFailed(false);
    try {
      const blob = await load(attachment.attachment_id);
      const url = URL.createObjectURL(blob);
      if (image) {
        setPreviewUrl(url);
        return;
      }
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = name;
      anchor.click();
      // The click only starts the download; revoking on the next turn of the
      // loop keeps the URL alive long enough for it to begin.
      setTimeout(() => URL.revokeObjectURL(url), 0);
    } catch {
      setFailed(true);
    } finally {
      setBusy(false);
    }
  }

  return (
    <li
      className="message-attachment"
      data-availability={attachment.availability}
      data-degraded={attachment.degradation ? "true" : undefined}
    >
      {previewUrl ? (
        <img className="message-attachment__image" src={previewUrl} alt={name} />
      ) : null}
      {unavailable ? (
        <span className="message-attachment__missing">{unavailable}</span>
      ) : (
        <button
          type="button"
          className="message-attachment__link"
          aria-expanded={image ? previewUrl !== null : undefined}
          aria-label={
            image
              ? previewUrl
                ? t("chat.attachmentHideImage", { name })
                : t("chat.attachmentShowImage", { name })
              : t("chat.attachmentSave", { name })
          }
          disabled={busy}
          onClick={() => void open()}
        >
          <FileIcon aria-hidden="true" />
          {label}
        </button>
      )}
      {degraded ? (
        <span className="message-attachment__degraded">{degraded}</span>
      ) : null}
      {failed ? (
        <span className="message-attachment__error" role="alert">
          {t("chat.attachmentLoadFailed")}
        </span>
      ) : null}
    </li>
  );
}

function MessageBubble({
  message,
  attachments = [],
  loadAttachment,
  retryContent = null,
  onRetry,
  onEdit,
  onFork,
  onBranch,
}: {
  message: ChatMessage;
  attachments?: readonly ProductMessageAttachmentRef[];
  loadAttachment?: (attachmentId: string) => Promise<Blob>;
  /** The prompt to resend; only the final answer carries it. */
  retryContent?: string | null;
  onRetry?: (content: string) => void;
  onEdit?: (content: string) => void;
  onFork?: () => void;
  onBranch?: () => void;
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
                : message.aborted === true
                  ? t("chat.aborted")
                  : ""}
            </span>
          </div>
          <RichText content={message.content} />
          <AttachmentRow attachments={attachments} loadAttachment={loadAttachment} />
          {message.aborted === true ? (
            // R2b smart stop: the text above is what the user had already read
            // when they stopped the turn. The byline keeps the compact status
            // tag; this is the reserved smart-stop copy that says what happened
            // to that text. It stays out of the body, which must remain exactly
            // the model's text for copying, editing, and forking.
            <p className="message-abort-note">
              {t("chat.smartStopPartialAbort")}
            </p>
          ) : null}
          <MessageActions
            content={message.content}
            retryContent={message.role === "assistant" ? retryContent : null}
            onRetry={onRetry}
            onEdit={message.role === "user" ? onEdit : undefined}
            onFork={message.role === "user" ? onFork : undefined}
            onBranch={message.role === "user" ? onBranch : undefined}
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
  // The head's elapsed value is measured from when this client first saw each
  // item's events, so it is unavailable (and hidden) for anything restored or
  // replayed. It ticks while the activity runs and freezes once it settles.
  const arrivalKeys = [
    ...entry.tools.map((tool) => toolArrivalKey(tool.id)),
    ...entry.inputs.map((input) => inputArrivalKey(input.id)),
  ];
  const [elapsedNow, setElapsedNow] = useState<number | null>(null);
  useEffect(() => {
    if (!inFlight) {
      setElapsedNow(null);
      return;
    }
    setElapsedNow(performance.now());
    const timer = window.setInterval(() => setElapsedNow(performance.now()), 1_000);
    return () => window.clearInterval(timer);
  }, [inFlight]);
  const timing = activityTiming(arrivalKeys, {
    active: inFlight,
    now: elapsedNow ?? 0,
    arrivals: toolArrivals,
  });
  const elapsedLabel =
    timing.liveMs !== null
      ? t("chat.activityElapsed", { n: elapsedSeconds(timing.liveMs) })
      : timing.frozenMs !== null
        ? t("chat.activityElapsedFrozen", { n: elapsedSeconds(timing.frozenMs) })
        : null;

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
                {elapsedLabel ? (
                  <span className="activity-group__elapsed">{elapsedLabel}</span>
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
  // The runtime's own measurement of an MCP call. A live arrival span is not a
  // substitute here: it also covers queueing on this client.
  const durationBadge = durationBadgeLabel(tool.protocolMetadata?.duration_ms);
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
        {durationBadge ? (
          <span className="tool-card__duration">{durationBadge}</span>
        ) : null}
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

/**
 * Fact labels are mostly data (diff summaries). Only the labels the shell
 * itself introduces are translated.
 */
function toolFactLabel(label: string, t: (path: string) => string): string {
  return label === "duration" ? t("chat.toolDuration") : label;
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
              <div key={index}><dt>{toolFactLabel(row.label, t)}</dt><dd>{row.value}</dd></div>
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
  onBranch,
}: {
  content: string;
  /** Prompt to resend; present only on the final assistant reply. */
  retryContent?: string | null;
  onRetry?: (content: string) => void;
  onEdit?: (content: string) => void;
  onFork?: () => void;
  onBranch?: () => void;
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
      {onBranch ? (
        // Labelled rather than icon-only: this is the destructive-looking
        // variant of edit-and-resend, and it must not be mistaken for the copy
        // or the terminal-boundary fork next to it.
        <button type="button" className="ghost" onClick={onBranch}>
          {t("chat.branchFromMessage")}
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
  const pruning = promptPruningSummary(message.promptBuild?.pruning);
  if (
    !message.usage &&
    !message.promptBuild &&
    !message.promptCompaction &&
    !pruning
  ) {
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
      {/* The pruning facts belong to the compaction request this turn made, so
          they sit next to the compaction row and say so. A build that pruned
          nothing carries no fields at all and renders nothing here. */}
      {pruning ? (
        <>
          {/* Rows are per fact class, not per object: a request that only left
              out whole messages carries no tool-elision count at all, and a row
              reading "0 条" would report a measurement the server never made.
              Each row is therefore gated on its own count — the byte widths are
              the sizes of those counted facts, so they never justify a row on
              their own. */}
          {pruning.elidedToolResults > 0 ? (
            <div>
              <dt>{t("inspector.prunedTools")}</dt>
              <dd>
                {t("inspector.prunedToolsValue", {
                  count: formatNumber(pruning.elidedToolResults),
                  size: formatPruningBytes(pruning.elidedBytes),
                })}
              </dd>
            </div>
          ) : null}
          {pruning.omittedMessages > 0 ? (
            <div>
              <dt>{t("inspector.prunedOmitted")}</dt>
              <dd>
                {t("inspector.prunedOmittedValue", {
                  count: formatNumber(pruning.omittedMessages),
                  size: formatPruningBytes(pruning.omittedBytes),
                })}
              </dd>
            </div>
          ) : null}
          {pruning.policy !== null ? (
            <div>
              <dt>{t("inspector.pruningPolicy")}</dt>
              <dd>
                {pruning.policyKnown
                  ? pruning.policy
                  : t("inspector.pruningPolicyUnknown")}
              </dd>
            </div>
          ) : null}
          {pruning.digests.length > 0 ? (
            <div>
              <dt>{t("inspector.pruningDigests")}</dt>
              <dd>
                {/* Identities, not links: they correlate with the trace and
                    open nothing. Collapsed because eight hashes are noise to
                    a reader who is not chasing one. */}
                <details className="message-evidence__digests">
                  <summary>
                    {t("inspector.pruningDigestsValue", {
                      count: formatNumber(pruning.digests.length),
                    })}
                  </summary>
                  <ul>
                    {pruning.digests.map((digest) => (
                      <li key={digest}>
                        <code>{digest}</code>
                      </li>
                    ))}
                  </ul>
                </details>
              </dd>
            </div>
          ) : null}
          <div>
            <dt>{t("inspector.pruningScope")}</dt>
            <dd>{t("inspector.pruningScopeValue")}</dd>
          </div>
        </>
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
