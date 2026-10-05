"use client";

import { useCallback, useEffect, useReducer, useRef, useState } from "react";

import {
  createRunController,
  describeError,
  isRunControllerInactive,
} from "../api/run-controller";
import {
  createWorkbenchState,
  workbenchReducer,
  type ToolCallView,
  type WorkbenchAction,
} from "../lib/rove-state";
import {
  IDLE_PHASE,
  activityEventForStreamEvent,
  reduceActivityPhase,
  type ActivityEvent,
  type ActivityPhase,
} from "../chat/activity-phase";
import { captureActiveSessionView } from "../chat/session-view-snapshot";
import { recordArrivalsForAction } from "../chat/tool-timing";
import type { FollowupConfirmationResult } from "../chat/followup-recovery";
import {
  MAX_PRODUCT_TRANSCRIPT_PAGE_RUNS,
  type ProductControl,
  type ProductControlKind,
  type ProductMessage,
  type ProductMessageAttachmentRequest,
  type ProductMessageDelivery,
  type ProductTranscriptResponse,
  type ProductTranscriptRunSegment,
} from "../product/product-api-types";
import { ProductApiError, type ProductApiClient } from "../product/product-client";
import {
  NO_OLDER_HISTORY,
  type OlderHistoryState,
} from "../chat/transcript-window";
import {
  findSession,
  updateSession,
  type ProductCatalog,
} from "./product-catalog";
import type {
  ActiveProviderSelection,
  ProviderProfileRecord,
  SessionRecord,
} from "./product-types";
import {
  loadedTranscriptHistory,
  prependTranscriptPage,
  projectProductTranscript,
  transcriptProjectionInput,
  type LoadedTranscriptHistory,
  type TranscriptRestoreState,
} from "./transcript-projection";
import {
  assertProviderSelectionIsSatisfiable,
} from "./turn-request";
import { autoSessionTitle, hasDefaultSessionTitle } from "./session-auto-title";
import { hasAdvancedRuntimeBinding } from "./server-product-state";

interface ObservedRunBinding {
  jobId: string;
  runId: string | null;
}

export function useSessionContinuity({
  productClient,
  catalogRef,
  activeSession,
  selection,
  profiles,
  markSession,
  refreshSessionStatuses,
  updateSessionTitle,
  setConnection,
  onTurnTerminal,
}: {
  productClient: ProductApiClient;
  catalogRef: { current: ProductCatalog };
  activeSession: SessionRecord | null;
  selection: ActiveProviderSelection;
  profiles: ProviderProfileRecord[];
  markSession: (
    sessionId: string,
    patch: Parameters<typeof updateSession>[2],
  ) => void;
  refreshSessionStatuses: () => Promise<boolean>;
  updateSessionTitle: (sessionId: string, title: string) => Promise<void>;
  setConnection: (connection: "unknown" | "ok" | "error") => void;
  /**
   * The focused session's turn settled and no successor took over, at the same
   * moment the terminal reconciliation refreshes it. The composer uses this to
   * drop a send snapshot that a later stop could no longer legitimately restore
   * (design F6).
   */
  onTurnTerminal?: (sessionId: string, workspaceId: string) => void;
}) {
  const [restoreState, setRestoreState] = useState<TranscriptRestoreState>({
    status: "idle",
  });
  const onTurnTerminalRef = useRef(onTurnTerminal);
  useEffect(() => {
    onTurnTerminalRef.current = onTurnTerminal;
  }, [onTurnTerminal]);
  const [olderHistory, setOlderHistory] =
    useState<OlderHistoryState>(NO_OLDER_HISTORY);
  const [approvalBusyKey, setApprovalBusyKey] = useState<string | null>(null);
  const [approvalError, setApprovalError] = useState<string | null>(null);
  const approvalRequestsRef = useRef(new Set<string>());
  const [inputBusy, setInputBusy] = useState<string | null>(null);
  const [controls, setControls] = useState<ProductControl[]>([]);
  const [messages, setMessages] = useState<ProductMessage[]>([]);
  const [controlsLoading, setControlsLoading] = useState(false);
  const [controlBusy, setControlBusy] = useState<string | null>(null);
  const [controlError, setControlError] = useState<string | null>(null);
  const [runState, baseDispatch] = useReducer(
    workbenchReducer,
    undefined,
    createWorkbenchState,
  );
  // The composer's waiting line is a second, tiny projection of the same
  // dispatched facts. Every state change passes through this wrapper, so the
  // phase never sees a fact the reducer did not.
  const [activityPhase, trackActivityPhase] = useReducer(
    reduceActivityPhase,
    IDLE_PHASE,
  );
  const dispatch = useCallback(
    (action: WorkbenchAction) => {
      baseDispatch(action);
      const phaseEvent = activityEventForWorkbenchAction(action);
      if (phaseEvent) {
        trackActivityPhase(phaseEvent);
      }
      // Canonical events carry no timestamps, so an activity duration can only
      // be measured from when its events actually arrived here. Replays are
      // ignored inside the tracker rather than guessed at on the render path.
      recordArrivalsForAction(action, performance.now());
    },
    [baseDispatch],
  );
  const approvalContextRef = useRef({ activeSession, runState });
  approvalContextRef.current = { activeSession, runState };
  const controllerRef = useRef<ReturnType<typeof createRunController> | null>(null);
  // The send callback is stable, so the provider guard reads selection and
  // profiles through refs instead of capturing a stale render's values.
  const selectionRef = useRef(selection);
  selectionRef.current = selection;
  const profilesRef = useRef(profiles);
  profilesRef.current = profiles;
  const focusedSessionRef = useRef<string | null>(null);
  const restoredSessionRef = useRef<string | null>(null);
  const observedBindingRef = useRef<ObservedRunBinding | null>(null);
  const transcriptGenerationRef = useRef(0);
  // The pages already loaded, so an older page can be prepended and the whole
  // history re-projected through the canonical projection.
  const loadedTranscriptRef = useRef<LoadedTranscriptHistory | null>(null);
  // The cursor the server published for the last page. The server covers every
  // run at or above it, including runs it could not read, so following it makes
  // progress even when a whole page resolves to partial reasons.
  const olderCursorRef = useRef<number | null>(null);
  const olderHistoryRequestRef = useRef(false);
  const controlsGenerationRef = useRef(0);
  const messagesGenerationRef = useRef(0);
  const messageRequestsRef = useRef(new Map<string, string>());
  /**
   * F10.2: sessions this page has already auto-titled. The catalog check below
   * is the real guard; this only stops a second message, sent before the
   * server's title write came back, from replacing the first message's title.
   */
  const autoTitledSessionsRef = useRef(new Set<string>());
  const controlRequestsRef = useRef(
    new Map<
      string,
      { idempotencyKey: string; kind: ProductControlKind; sessionId: string }
    >(),
  );
  const refreshStatusesRef = useRef(refreshSessionStatuses);
  /**
   * The run binding this focused session has already watched reach a terminal
   * state.
   *
   * Attaching to a job that is already terminal reports terminal immediately
   * (`api/run-controller.ts`), and each controller remembers that only for
   * itself. Re-attaching to the binding that just ended therefore starts the
   * whole terminal reconciliation again — reconcile → restore → attach →
   * terminal → reconcile — and it stops only when every other authority agrees
   * the run is over. When a transcript snapshot still calls that run live (a read
   * that raced the cancel, or a server that has not yet published the terminal
   * status), nothing agreed and the shell re-attached without end: measured at
   * 88–175 transcript reads in six seconds after a single stop, and 3 when the
   * snapshot happened to be current.
   *
   * The binding is remembered here and compared as a pair, because a successor
   * run can reuse the job id and only changes the run id; a successor is still
   * followed. A different session's binding never compares equal, so switching
   * sessions needs no clearing.
   */
  const terminatedBindingRef = useRef<{
    sessionId: string;
    jobId: string;
    runId: string | null;
  } | null>(null);
  const terminalReconciliationRef = useRef<
    (
      sessionId: string,
      controller: ReturnType<typeof createRunController>,
      completedBinding: ObservedRunBinding | null,
    ) => void
  >(() => undefined);

  useEffect(() => {
    refreshStatusesRef.current = refreshSessionStatuses;
  }, [refreshSessionStatuses]);

  const clearControls = useCallback(() => {
    ++controlsGenerationRef.current;
    setApprovalBusyKey(null);
    setApprovalError(null);
    setControls([]);
    setControlsLoading(false);
    setControlBusy(null);
    setControlError(null);
    ++messagesGenerationRef.current;
    setMessages([]);
    loadedTranscriptRef.current = null;
    olderCursorRef.current = null;
    olderHistoryRequestRef.current = false;
    setOlderHistory(NO_OLDER_HISTORY);
  }, []);

  const refreshMessages = useCallback(
    async (sessionId: string): Promise<ProductMessage[] | null> => {
      const generation = ++messagesGenerationRef.current;
      try {
        const response = await productClient.listMessages(sessionId);
        if (
          messagesGenerationRef.current !== generation ||
          focusedSessionRef.current !== sessionId
        ) {
          return null;
        }
        setMessages(response.messages);
        return response.messages;
      } catch (error) {
        if (focusedSessionRef.current === sessionId) {
          setControlError(`Could not refresh messages: ${describeError(error)}`);
        }
        return null;
      }
    },
    [productClient],
  );

  const upsertMessage = useCallback((message: ProductMessage) => {
    if (focusedSessionRef.current !== message.product_session_id) {
      return;
    }
    setMessages((current) => {
      const rest = current.filter((item) => item.id !== message.id);
      return [...rest, message].sort((left, right) => left.seq - right.seq);
    });
  }, []);

  const refreshControls = useCallback(
    async (sessionId: string): Promise<ProductControl[] | null> => {
      const generation = ++controlsGenerationRef.current;
      setControlsLoading(true);
      try {
        const response = await productClient.listControls(sessionId);
        if (
          controlsGenerationRef.current !== generation ||
          focusedSessionRef.current !== sessionId
        ) {
          return null;
        }
        setControls(response.controls);
        setControlError(null);
        return response.controls;
      } catch (error) {
        if (
          controlsGenerationRef.current === generation &&
          focusedSessionRef.current === sessionId
        ) {
          setControlError(`Could not refresh controls: ${describeError(error)}`);
        }
        return null;
      } finally {
        if (controlsGenerationRef.current === generation) {
          setControlsLoading(false);
        }
      }
    },
    [productClient],
  );

  const upsertControl = useCallback((control: ProductControl) => {
    if (focusedSessionRef.current !== control.product_session_id) {
      return;
    }
    setControls((current) => {
      const index = current.findIndex((item) => item.id === control.id);
      if (index === -1) {
        return [...current, control].sort((left, right) => left.seq - right.seq);
      }
      return [
        ...current.slice(0, index),
        control,
        ...current.slice(index + 1),
      ];
    });
  }, []);

  const closeFocusedObservation = useCallback(() => {
    controllerRef.current?.close();
    controllerRef.current = null;
    observedBindingRef.current = null;
  }, []);

  const installFocusedController = useCallback(
    (sessionId: string) => {
      closeFocusedObservation();
      let controller: ReturnType<typeof createRunController>;
      controller = createRunController(dispatch, {
        onTerminal: () => {
          if (
            controllerRef.current !== controller ||
            focusedSessionRef.current !== sessionId
          ) {
            return;
          }
          const completedBinding = observedBindingRef.current;
          observedBindingRef.current = null;
          if (completedBinding) {
            // Remembered even if the reconciliation below finds a successor: the
            // successor has its own run id, so this only refuses the run that
            // has already ended.
            terminatedBindingRef.current = {
              sessionId,
              jobId: completedBinding.jobId,
              runId: completedBinding.runId,
            };
          }
          terminalReconciliationRef.current(sessionId, controller, completedBinding);
        },
        onStreamEvent: (event) => {
          if (
            focusedSessionRef.current !== sessionId ||
            (event.type !== "steer_accepted" &&
              event.type !== "steer_applied" &&
              event.type !== "steer_dropped" &&
              event.type !== "followup_queued" &&
              event.type !== "followup_dequeued" &&
              event.type !== "followup_abandoned" &&
              event.type !== "message_queued" &&
              event.type !== "message_intervention_requested" &&
              event.type !== "message_applied_current_run" &&
              event.type !== "message_claimed_successor" &&
              event.type !== "message_needs_attention" &&
              event.type !== "message_revoked")
          ) {
            return;
          }
          void refreshControls(sessionId);
          void refreshMessages(sessionId);
        },
      });
      controllerRef.current = controller;
      return controller;
    },
    [closeFocusedObservation, refreshControls, refreshMessages],
  );

  const attachFocusedJob = useCallback(
    (sessionId: string, jobId: string, runId: string | null) => {
      const observed = observedBindingRef.current;
      const terminated = terminatedBindingRef.current;
      if (
        focusedSessionRef.current !== sessionId ||
        (observed?.jobId === jobId && observed.runId === runId) ||
        // This run has already ended in this window. A terminal job publishes
        // nothing further, so following it again can only re-run the
        // reconciliation that ended it — the durable restore is the way back to
        // its facts.
        (terminated?.sessionId === sessionId &&
          terminated.jobId === jobId &&
          terminated.runId === runId)
      ) {
        return;
      }
      const controller = installFocusedController(sessionId);
      observedBindingRef.current = { jobId, runId };
      dispatch({ type: "prepare_job_attachment", jobId, runId });
      void controller.attach(jobId, runId).catch((error) => {
        if (
          isRunControllerInactive(error) ||
          focusedSessionRef.current !== sessionId ||
          controllerRef.current !== controller
        ) {
          return;
        }
        const detail = `Live follow could not reconnect: ${describeError(error)}. Durable transcript restore remains available.`;
        dispatch({ type: "set_error", error: detail });
        setRestoreState({ status: "error", sessionId, error: detail });
      });
    },
    [installFocusedController],
  );

  const restoreSession = useCallback(
    async (workspaceId: string, sessionId: string) => {
      const generation = ++transcriptGenerationRef.current;
      closeFocusedObservation();
      focusedSessionRef.current = sessionId;
      restoredSessionRef.current = sessionId;
      clearControls();
      dispatch({ type: "reset" });
      setRestoreState({ status: "loading", sessionId });

      try {
        const transcript = await productClient.getTranscript(sessionId, {
          limitRuns: MAX_PRODUCT_TRANSCRIPT_PAGE_RUNS,
        });
        if (
          transcriptGenerationRef.current !== generation ||
          focusedSessionRef.current !== sessionId
        ) {
          return;
        }
        if (transcript.workspace_id !== workspaceId) {
          throw new Error("Transcript workspace does not match the session route.");
        }
        const history = loadedTranscriptHistory(transcript);
        loadedTranscriptRef.current = history;
        const older = olderHistoryFromResponse(transcript);
        olderCursorRef.current = older.cursor;
        setOlderHistory(older);
        const projected = projectProductTranscript(
          transcriptProjectionInput(history),
        );
        dispatch({ type: "hydrate", state: projected });
        setConnection("ok");
        setRestoreState(
          transcript.status === "partial"
            ? {
                status: "partial",
                sessionId,
                reasons: transcript.partial_reasons,
              }
            : { status: "complete", sessionId },
        );
        void refreshControls(sessionId);
        void refreshMessages(sessionId);

        const session = findSession(catalogRef.current, sessionId);
        const liveJobId =
          projected.busy && projected.activeJobId
            ? projected.activeJobId
            : session &&
                (session.status === "running" || session.status === "needs_attention")
              ? session.activeJobId ?? null
              : null;
        const liveRunId =
          projected.busy && projected.activeJobId
            ? projected.activeRunId
            : session &&
                (session.status === "running" || session.status === "needs_attention")
              ? session.activeRunId ?? null
              : null;
        if (liveJobId) {
          attachFocusedJob(sessionId, liveJobId, liveRunId);
        }
      } catch (error) {
        if (
          transcriptGenerationRef.current !== generation ||
          focusedSessionRef.current !== sessionId
        ) {
          return;
        }
        dispatch({ type: "reset" });
        setConnection("error");
        setRestoreState({
          status: "error",
          sessionId,
          error: describeError(error),
        });
      }
    },
    [
      attachFocusedJob,
      catalogRef,
      clearControls,
      closeFocusedObservation,
      productClient,
      refreshControls,
      refreshMessages,
    ],
  );

  /**
   * Fetch the next older page and prepend it to the loaded history. The whole
   * loaded history is re-projected through the canonical projection, so a
   * cursor page is never a second transcript state machine.
   */
  const loadOlderHistory = useCallback(async () => {
    const sessionId = focusedSessionRef.current;
    const loaded = loadedTranscriptRef.current;
    if (!sessionId || !loaded || loaded.segments.length === 0) {
      return;
    }
    if (olderHistoryRequestRef.current) {
      return;
    }
    // Prefer the cursor the server published: it stays strictly older than
    // every run the page covered, including runs that only produced partial
    // reasons. The oldest loaded segment is the fallback for a cursor-less
    // response.
    const beforeOrdinal =
      olderCursorRef.current ?? oldestLoadedOrdinal(loaded.segments);
    if (beforeOrdinal === null || beforeOrdinal <= 1) {
      // Ordinals start at 1: there is nothing strictly older to ask for.
      olderCursorRef.current = null;
      setOlderHistory((current) => ({
        ...current,
        hasMore: false,
        cursor: null,
        loading: false,
        error: null,
      }));
      return;
    }
    const generation = transcriptGenerationRef.current;
    olderHistoryRequestRef.current = true;
    setOlderHistory((current) => ({ ...current, loading: true, error: null }));
    try {
      const page = await productClient.getTranscript(sessionId, {
        beforeOrdinal,
        limitRuns: MAX_PRODUCT_TRANSCRIPT_PAGE_RUNS,
      });
      if (
        transcriptGenerationRef.current !== generation ||
        focusedSessionRef.current !== sessionId
      ) {
        return;
      }
      if (page.workspace_id !== loaded.workspaceId) {
        throw new Error("Older transcript page does not match the session route.");
      }
      const merged = prependTranscriptPage(loaded, page);
      loadedTranscriptRef.current = merged;
      dispatch({
        type: "hydrate",
        state: projectProductTranscript(transcriptProjectionInput(merged)),
      });
      const older = olderHistoryFromResponse(page);
      olderCursorRef.current = older.cursor;
      setOlderHistory(older);
      if (merged.partialReasons.length > 0) {
        setRestoreState({
          status: "partial",
          sessionId,
          reasons: merged.partialReasons,
        });
      }
    } catch (error) {
      if (
        transcriptGenerationRef.current !== generation ||
        focusedSessionRef.current !== sessionId
      ) {
        return;
      }
      // The cursor stays published, so the reader can retry the same page.
      setOlderHistory((current) => ({
        ...current,
        loading: false,
        error: boundErrorMessage(
          `Could not load older history: ${describeError(error)}`,
        ),
      }));
    } finally {
      olderHistoryRequestRef.current = false;
    }
  }, [dispatch, productClient]);

  const focusSession = useCallback(
    (workspaceId: string, sessionId: string) => {
      if (restoredSessionRef.current !== sessionId) {
        // A route change (deep link, back/forward) replaces the transcript
        // without the prepare step, so the view of the session on screen is
        // captured here — it is still intact at this point.
        captureActiveSessionView();
        void restoreSession(workspaceId, sessionId);
      }
    },
    [restoreSession],
  );

  const prepareSession = useCallback(
    (sessionId: string) => {
      captureActiveSessionView();
      ++transcriptGenerationRef.current;
      closeFocusedObservation();
      focusedSessionRef.current = sessionId;
      restoredSessionRef.current = null;
      clearControls();
      dispatch({ type: "reset" });
      setRestoreState({ status: "loading", sessionId });
    },
    [clearControls, closeFocusedObservation],
  );

  const leaveSession = useCallback(() => {
    captureActiveSessionView();
    ++transcriptGenerationRef.current;
    closeFocusedObservation();
    focusedSessionRef.current = null;
    restoredSessionRef.current = null;
    clearControls();
    dispatch({ type: "reset" });
    setRestoreState({ status: "idle" });
  }, [clearControls, closeFocusedObservation]);

  const retryRestore = useCallback(
    (workspaceId: string, sessionId: string) => {
      restoredSessionRef.current = null;
      void restoreSession(workspaceId, sessionId);
    },
    [restoreSession],
  );

  const submitControl = useCallback(
    async (kind: ProductControlKind, content: string): Promise<boolean> => {
      const sessionId = focusedSessionRef.current ?? catalogRef.current.active.sessionId;
      const session = sessionId ? findSession(catalogRef.current, sessionId) : null;
      const trimmed = content.trim();
      if (!session || !trimmed) {
        setControlError("Open a session and enter a control message first.");
        return false;
      }

      const requestKey = `${session.id}\u0000${kind}\u0000${trimmed}`;
      let pending = controlRequestsRef.current.get(requestKey);
      if (!pending) {
        if (controlRequestsRef.current.size >= MAX_PENDING_CONTROL_REQUESTS) {
          const oldest = controlRequestsRef.current.keys().next().value;
          if (oldest) {
            controlRequestsRef.current.delete(oldest);
          }
        }
        pending = {
          idempotencyKey: createControlIdempotencyKey(),
          kind,
          sessionId: session.id,
        };
        controlRequestsRef.current.set(requestKey, pending);
      }

      const busyId = `submit:${kind}`;
      setControlBusy(busyId);
      setControlError(null);
      try {
        const control =
          kind === "steer"
            ? await productClient.enqueueSteer(session.id, {
                content: trimmed,
                idempotency_key: pending.idempotencyKey,
              })
            : await productClient.enqueueFollowup(session.id, {
                content: trimmed,
                idempotency_key: pending.idempotencyKey,
              });
        controlRequestsRef.current.delete(requestKey);
        if (focusedSessionRef.current !== session.id) {
          return true;
        }
        upsertControl(control);
        setConnection("ok");
        void refreshControls(session.id);
        return true;
      } catch (error) {
        if (focusedSessionRef.current !== session.id) {
          return false;
        }
        const detail = describeError(error);
        setControlError(`Could not submit ${controlLabel(kind)}: ${detail}`);
        if (isLikelyNetworkError(detail)) {
          setConnection("error");
        }
        return false;
      } finally {
        if (focusedSessionRef.current === session.id) {
          setControlBusy((current) => (current === busyId ? null : current));
        }
      }
    },
    [catalogRef, productClient, refreshControls, setConnection, upsertControl],
  );

  const revokeControl = useCallback(
    async (controlId: string) => {
      const sessionId = focusedSessionRef.current ?? catalogRef.current.active.sessionId;
      if (!sessionId) {
        setControlError("Open the session that owns this control first.");
        return;
      }
      const busyId = `revoke:${controlId}`;
      setControlBusy(busyId);
      setControlError(null);
      try {
        const control = await productClient.revokeControl(sessionId, controlId);
        if (focusedSessionRef.current !== sessionId) {
          return;
        }
        upsertControl(control);
        setConnection("ok");
        void refreshControls(sessionId);
      } catch (error) {
        if (focusedSessionRef.current !== sessionId) {
          return;
        }
        setControlError(`Could not revoke control: ${describeError(error)}`);
      } finally {
        if (focusedSessionRef.current === sessionId) {
          setControlBusy((current) => (current === busyId ? null : current));
        }
      }
    },
    [catalogRef, productClient, refreshControls, setConnection, upsertControl],
  );

  /**
   * Retry a successor the server abandoned.
   *
   * The confirm is a queue fact, not a ledger fact: the route returns the
   * control to `pending`, moves the session back off `needs_attention`, and
   * re-attempts the start itself, so the message row and the session status both
   * change without a frame of their own. Both are therefore re-read from the
   * server rather than assumed — including the status, because the sidebar badge
   * and the composer's send gate read it and would otherwise keep claiming the
   * session needs attention after the server said it does not.
   *
   * The settled answer is returned rather than also written to `controlError`:
   * the row that owns the action renders it, and reporting one refusal on two
   * surfaces is the double-report the notification layer already avoids.
   */
  const confirmFollowup = useCallback(
    async (controlId: string): Promise<FollowupConfirmationResult> => {
      const sessionId = focusedSessionRef.current ?? catalogRef.current.active.sessionId;
      if (!sessionId) {
        setControlError("Open the session that owns this follow-up first.");
        return { ok: false, reason: "no session is open" };
      }
      const busyId = `confirm:${controlId}`;
      setControlBusy(busyId);
      setControlError(null);
      try {
        const control = await productClient.confirmFollowup(sessionId, controlId);
        if (focusedSessionRef.current !== sessionId) {
          return { ok: true, controlStatus: control.status };
        }
        upsertControl(control);
        setConnection("ok");
        void refreshControls(sessionId);
        void refreshMessages(sessionId);
        void refreshStatusesRef.current();
        return { ok: true, controlStatus: control.status };
      } catch (error) {
        const reason = describeError(error);
        if (focusedSessionRef.current !== sessionId) {
          return { ok: false, reason };
        }
        // The ledger is re-read even on the refusal path: the server's reason is
        // usually that the session moved underneath this request (a turn became
        // active), and that move is itself worth showing.
        void refreshMessages(sessionId);
        if (isLikelyNetworkError(reason)) {
          setConnection("error");
        }
        return { ok: false, reason };
      } finally {
        if (focusedSessionRef.current === sessionId) {
          setControlBusy((current) => (current === busyId ? null : current));
        }
      }
    },
    [
      catalogRef,
      productClient,
      refreshControls,
      refreshMessages,
      setConnection,
      upsertControl,
    ],
  );

  /**
   * R4: promote one queued message with an explicit delivery intent.
   *
   * `successor` moves it to the head of the successor queue and waits for the
   * current turn's terminal boundary; `current_run` steers the live turn
   * instead. The typed failure code is returned rather than only logged, because
   * the two intents fail for different user-visible reasons ("the current turn
   * may already have ended").
   */
  const promoteMessage = useCallback(
    async (
      messageId: string,
      delivery?: ProductMessageDelivery,
    ): Promise<{ ok: boolean; code: string | null }> => {
      const sessionId = focusedSessionRef.current ?? catalogRef.current.active.sessionId;
      if (!sessionId) {
        return { ok: false, code: null };
      }
      const busyId = `promote:${messageId}`;
      setControlBusy(busyId);
      setControlError(null);
      try {
        upsertMessage(
          await productClient.promoteMessage(
            sessionId,
            messageId,
            delivery === undefined ? undefined : { delivery },
          ),
        );
        void refreshMessages(sessionId);
        return { ok: true, code: null };
      } catch (error) {
        setControlError(`Could not request intervention: ${describeError(error)}`);
        void refreshMessages(sessionId);
        return {
          ok: false,
          code: error instanceof ProductApiError ? error.code : null,
        };
      } finally {
        setControlBusy((current) => (current === busyId ? null : current));
      }
    },
    [catalogRef, productClient, refreshMessages, upsertMessage],
  );

  /**
   * R4: rewrite the successor queue in one atomic request.
   *
   * `orderedIds` must name exactly the successor messages the session currently
   * holds; a stale list is refused with a typed conflict rather than merged, so
   * a concurrent promote, revoke, or drain cannot be silently overwritten. The
   * refused request is followed by a refresh, so the caller can show the
   * server's order instead of the order it had hoped for.
   */
  const reorderMessages = useCallback(
    async (
      orderedIds: readonly string[],
    ): Promise<{ ok: boolean; code: string | null }> => {
      const sessionId = focusedSessionRef.current ?? catalogRef.current.active.sessionId;
      if (!sessionId) {
        return { ok: false, code: null };
      }
      const busyId = "reorder-messages";
      setControlBusy(busyId);
      setControlError(null);
      try {
        const response = await productClient.reorderMessages(sessionId, {
          ordered_ids: [...orderedIds],
        });
        for (const message of response.messages) {
          upsertMessage(message);
        }
        void refreshMessages(sessionId);
        return { ok: true, code: null };
      } catch (error) {
        setControlError(`Could not reorder the queue: ${describeError(error)}`);
        void refreshMessages(sessionId);
        return {
          ok: false,
          code: error instanceof ProductApiError ? error.code : null,
        };
      } finally {
        setControlBusy((current) => (current === busyId ? null : current));
      }
    },
    [catalogRef, productClient, refreshMessages, upsertMessage],
  );

  const revokeMessage = useCallback(
    async (messageId: string) => {
      const sessionId = focusedSessionRef.current ?? catalogRef.current.active.sessionId;
      if (!sessionId) {
        return;
      }
      const busyId = `revoke-message:${messageId}`;
      setControlBusy(busyId);
      setControlError(null);
      try {
        upsertMessage(await productClient.revokeMessage(sessionId, messageId));
        void refreshMessages(sessionId);
      } catch (error) {
        setControlError(`Could not revoke message: ${describeError(error)}`);
        void refreshMessages(sessionId);
      } finally {
        setControlBusy((current) => (current === busyId ? null : current));
      }
    },
    [catalogRef, productClient, refreshMessages, upsertMessage],
  );

  const reconcileTerminal = useCallback(
    async (
      sessionId: string,
      controller: ReturnType<typeof createRunController>,
      completedBinding: ObservedRunBinding | null,
    ) => {
      for (const delayMs of TERMINAL_RECONCILIATION_DELAYS_MS) {
        if (delayMs > 0) {
          await waitForReconciliationDelay(delayMs);
        }
        if (
          controllerRef.current !== controller ||
          focusedSessionRef.current !== sessionId
        ) {
          return;
        }

        const refreshed = await refreshStatusesRef.current();
        if (
          !refreshed ||
          controllerRef.current !== controller ||
          focusedSessionRef.current !== sessionId
        ) {
          continue;
        }
        const currentControls = await refreshControls(sessionId);
        if (
          controllerRef.current !== controller ||
          focusedSessionRef.current !== sessionId
        ) {
          return;
        }
        const currentSession = findSession(catalogRef.current, sessionId);
        const successorJobId = currentSession?.activeJobId ?? null;
        const successorRunId = currentSession?.activeRunId ?? null;
        const successorIsLive =
          (currentSession?.status === "running" ||
            currentSession?.status === "needs_attention") &&
          successorJobId !== null &&
          successorRunId !== null &&
          (completedBinding === null ||
            successorJobId !== completedBinding.jobId ||
            successorRunId !== completedBinding.runId);
        if (successorIsLive) {
          attachFocusedJob(sessionId, successorJobId, successorRunId);
          return;
        }

        const followupStillPending = currentControls?.some(
          (control) =>
            control.kind === "followup" &&
            (control.status === "pending" || control.status === "accepted"),
        );
        if (followupStillPending || currentSession?.status === "running") {
          continue;
        }

        if (currentSession) {
          restoredSessionRef.current = null;
          onTurnTerminalRef.current?.(sessionId, currentSession.workspaceId);
          await restoreSession(currentSession.workspaceId, sessionId);
          return;
        }
      }

      const currentSession = findSession(catalogRef.current, sessionId);
      if (
        currentSession &&
        controllerRef.current === controller &&
        focusedSessionRef.current === sessionId
      ) {
        restoredSessionRef.current = null;
        onTurnTerminalRef.current?.(sessionId, currentSession.workspaceId);
        await restoreSession(currentSession.workspaceId, sessionId);
      }
    },
    [attachFocusedJob, catalogRef, refreshControls, restoreSession],
  );

  useEffect(() => {
    terminalReconciliationRef.current = (sessionId, controller, completedBinding) => {
      void reconcileTerminal(sessionId, controller, completedBinding);
    };
    return () => {
      terminalReconciliationRef.current = () => undefined;
    };
  }, [reconcileTerminal]);

  const reconcileAcceptedSuccessor = useCallback(
    async (previousSession: SessionRecord) => {
      dispatch({ type: "set_status", statusText: "Reconciling durable session" });
      for (const delayMs of AMBIGUOUS_START_RECONCILIATION_DELAYS_MS) {
        if (delayMs > 0) {
          await waitForReconciliationDelay(delayMs);
        }
        if (focusedSessionRef.current !== previousSession.id) {
          return false;
        }

        const refreshed = await refreshStatusesRef.current();
        if (
          !refreshed ||
          focusedSessionRef.current !== previousSession.id
        ) {
          continue;
        }
        const currentSession = findSession(
          catalogRef.current,
          previousSession.id,
        );
        if (!hasAdvancedRuntimeBinding(previousSession, currentSession)) {
          continue;
        }
        attachFocusedJob(
          previousSession.id,
          currentSession.activeJobId,
          currentSession.activeRunId,
        );
        if (focusedSessionRef.current === previousSession.id) {
          setConnection("ok");
        }
        return true;
      }
      return false;
    },
    [attachFocusedJob, catalogRef, setConnection],
  );

  const send = useCallback(
    async (
      message: string,
      attachments: ProductMessageAttachmentRequest[] = [],
      /**
       * §6.1: Alt+Enter asks for the `current_run` delivery. The send endpoint
       * has no delivery field — a message created during a live turn always
       * lands in the successor queue first — so an interjection is the send
       * followed by the promote request the queue UI already exposes. If the
       * promote is refused (the turn ended in between), the message stays
       * queued as a successor rather than being lost.
       */
      delivery?: ProductMessageDelivery,
    ): Promise<boolean> => {
      const session = findSession(
        catalogRef.current,
        catalogRef.current.active.sessionId,
      );
      if (!session) {
        dispatch({ type: "set_error", error: "Open a workspace and session first." });
        return false;
      }
      const trimmed = message.trim();
      if (!trimmed) {
        return false;
      }
      try {
        assertProviderSelectionIsSatisfiable(
          selectionRef.current,
          profilesRef.current,
        );
      } catch (error) {
        dispatch({ type: "set_error", error: describeError(error) });
        return false;
      }
      // The attachment ids are part of the request identity: the same text sent
      // with a different attachment set is a different message, and reusing the
      // idempotency key would make the server answer with the earlier one.
      const attachmentKey = attachments
        .map((attachment) => attachment.attachment_id)
        .sort()
        .join(",");
      const requestKey = `${session.id}\u0000${trimmed}\u0000${attachmentKey}`;
      let idempotencyKey = messageRequestsRef.current.get(requestKey);
      if (!idempotencyKey) {
        if (messageRequestsRef.current.size >= MAX_PENDING_MESSAGE_REQUESTS) {
          dispatch({
            type: "set_error",
            error: "Too many message submissions are awaiting reconciliation.",
          });
          return false;
        }
        idempotencyKey = createControlIdempotencyKey();
        messageRequestsRef.current.set(requestKey, idempotencyKey);
      }
      // F10.2: a session the user never named takes its first message as its
      // title. The value is computed here and written only once the server has
      // accepted the message, so a rejected send never names a session.
      const autoTitle = autoSessionTitle(trimmed);
      let accepted: ProductMessage | null = null;
      let lastError: unknown;
      trackActivityPhase({ type: "send" });
      for (const delayMs of MESSAGE_SUBMISSION_RETRY_DELAYS_MS) {
        if (delayMs > 0) {
          await waitForReconciliationDelay(delayMs);
        }
        if (focusedSessionRef.current !== session.id) {
          return false;
        }
        try {
          accepted = await productClient.sendMessage(session.id, {
            content: trimmed,
            idempotency_key: idempotencyKey,
            ...(attachments.length > 0 ? { attachments } : {}),
          });
          break;
        } catch (error) {
          lastError = error;
          if (!isAmbiguousJobStartError(error)) {
            break;
          }
          dispatch({
            type: "set_status",
            statusText: "Confirming whether the server accepted this message",
          });
        }
      }
      if (!accepted) {
        const detail = describeError(lastError);
        if (!isAmbiguousJobStartError(lastError)) {
          messageRequestsRef.current.delete(requestKey);
        }
        dispatch({ type: "set_error", error: `Could not send message: ${detail}` });
        if (isLikelyNetworkError(detail)) {
          setConnection("error");
        }
        return false;
      }

      messageRequestsRef.current.delete(requestKey);
      upsertMessage(accepted);
      if (delivery === "current_run" && accepted.status === "queued") {
        // Composed interjection: the message is durable in the queue, and this
        // second request asks for it to steer the live turn. `promoteMessage`
        // already reports a refusal through controlError.
        await promoteMessage(accepted.id, "current_run");
      }
      if (autoTitle !== null) {
        // Read the title again rather than trusting the snapshot this call
        // started with: a rename that landed while the message was in flight
        // must win, and it is exactly this check — "still the default template"
        // — that stands in for "never manually named" (design F10.2 records the
        // approximation). The write is fire-and-forget: the title is a
        // convenience, and the next catalog read is authoritative.
        const namedNow = findSession(catalogRef.current, session.id);
        if (
          !autoTitledSessionsRef.current.has(session.id) &&
          hasDefaultSessionTitle(namedNow?.title ?? session.title)
        ) {
          autoTitledSessionsRef.current.add(session.id);
          void updateSessionTitle(session.id, autoTitle).catch(() => undefined);
          markSession(session.id, { title: autoTitle });
        }
      }
      dispatch({ type: "set_status", statusText: messageStatusLabel(accepted.status) });
      setConnection("ok");
      void refreshMessages(session.id);
      const successorExpected =
        accepted.status === "claimed_successor" ||
        (accepted.status === "queued" && session.status === "idle");
      if (successorExpected) {
        const attached = await reconcileAcceptedSuccessor(session);
        if (!attached && focusedSessionRef.current === session.id) {
          await refreshMessages(session.id);
          setConnection("error");
          dispatch({
            type: "set_error",
            error:
              "The message was accepted, but its successor run binding is not visible yet. Reload before sending another message.",
          });
        }
      }
      return true;
    },
    [
      catalogRef,
      markSession,
      productClient,
      promoteMessage,
      reconcileAcceptedSuccessor,
      refreshMessages,
      setConnection,
      updateSessionTitle,
      upsertMessage,
    ],
  );

  const cancel = useCallback(async () => {
    if (!runState.activeJobId) {
      return;
    }
    try {
      await controllerRef.current?.cancel(runState.activeJobId);
    } catch (error) {
      if (!isRunControllerInactive(error)) {
        dispatch({ type: "set_error", error: describeError(error) });
      }
    }
  }, [runState.activeJobId]);

  const approve = useCallback(
    async (tool: ToolCallView, decision: "approve" | "reject") => {
      const { activeSession: session, runState: state } = approvalContextRef.current;
      const controller = controllerRef.current;
      const currentTool = state.tools.find((item) => item === tool);
      if (!session || focusedSessionRef.current !== session.id || !controller ||
          !state.activeJobId || !state.activeRunId || !currentTool?.pendingApproval) return;
      const key = JSON.stringify([session.workspaceId, session.id,
        state.activeJobId, state.activeRunId, tool.id]);
      if (approvalRequestsRef.current.has(key)) return;
      approvalRequestsRef.current.add(key);
      setApprovalBusyKey(key);
      setApprovalError(null);
      const isCurrent = () => {
        const current = approvalContextRef.current;
        return controllerRef.current === controller && focusedSessionRef.current === session.id &&
          current.activeSession?.id === session.id &&
          current.activeSession.workspaceId === session.workspaceId &&
          current.runState.activeJobId === state.activeJobId &&
          current.runState.activeRunId === state.activeRunId;
      };
      try {
        await controller.approve(state.activeJobId, tool.id, decision, state.activeRunId);
        if (isCurrent()) void refreshStatusesRef.current();
      } catch (error) {
        if (isCurrent() && !isRunControllerInactive(error)) {
          setApprovalError(describeError(error));
        }
      } finally {
        approvalRequestsRef.current.delete(key);
        if (isCurrent()) setApprovalBusyKey((current) => current === key ? null : current);
      }
    },
    [],
  );

  const answer = useCallback(
    async (inputId: string, value: string) => {
      if (!runState.activeJobId) {
        return;
      }
      setInputBusy(inputId);
      try {
        await controllerRef.current?.answer(runState.activeJobId, inputId, value);
      } catch (error) {
        if (!isRunControllerInactive(error)) {
          dispatch({ type: "set_error", error: describeError(error) });
        }
      } finally {
        setInputBusy(null);
      }
    },
    [runState.activeJobId],
  );

  useEffect(() => {
    if (!activeSession || focusedSessionRef.current !== activeSession.id) {
      return;
    }
    const waiting = runState.tools.some(
      (tool) => tool.pendingApproval || tool.status === "waiting",
    );
    if (waiting && activeSession.status !== "needs_attention") {
      markSession(activeSession.id, { status: "needs_attention" });
    } else if (!waiting && runState.busy && activeSession.status !== "running") {
      markSession(activeSession.id, { status: "running" });
    }
  }, [activeSession, markSession, runState.busy, runState.tools]);

  useEffect(() => {
    if (
      !activeSession ||
      focusedSessionRef.current !== activeSession.id ||
      (restoreState.status !== "complete" && restoreState.status !== "partial") ||
      (activeSession.status !== "running" &&
        activeSession.status !== "needs_attention") ||
      !activeSession.activeJobId ||
      !activeSession.activeRunId
    ) {
      return;
    }
    attachFocusedJob(
      activeSession.id,
      activeSession.activeJobId,
      activeSession.activeRunId,
    );
  }, [activeSession, attachFocusedJob, restoreState.status]);

  useEffect(
    () => () => {
      ++transcriptGenerationRef.current;
      closeFocusedObservation();
    },
    [closeFocusedObservation],
  );

  return {
    runState,
    activityPhase,
    restoreState,
    hasOlderHistory: olderHistory.hasMore,
    olderCursor: olderHistory.cursor,
    olderHistoryLoading: olderHistory.loading,
    olderHistoryError: olderHistory.error,
    loadOlderHistory,
    approvalBusy: runState.tools.find((tool) => approvalBusyKey === JSON.stringify([
      activeSession?.workspaceId, activeSession?.id, runState.activeJobId,
      runState.activeRunId, tool.id,
    ]))?.id ?? null,
    approvalError,
    inputBusy,
    controls,
    messages,
    controlsLoading,
    controlBusy,
    controlError,
    refreshControls,
    refreshMessages,
    focusSession,
    prepareSession,
    leaveSession,
    retryRestore,
    send,
    submitSteer: (content: string) => submitControl("steer", content),
    submitFollowup: (content: string) => submitControl("followup", content),
    revokeControl,
    confirmFollowup,
    promoteMessage,
    reorderMessages,
    revokeMessage,
    cancel,
    approve,
    answer,
  };
}

const MAX_PENDING_CONTROL_REQUESTS = 32;
const MAX_PENDING_MESSAGE_REQUESTS = 32;
const MESSAGE_SUBMISSION_RETRY_DELAYS_MS = [0, 100, 200, 400] as const;
const TERMINAL_RECONCILIATION_DELAYS_MS = [0, 100, 200, 400, 800, 1_000] as const;

function createControlIdempotencyKey(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return `control_${crypto.randomUUID()}`;
  }
  return `control_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 12)}`;
}

const TERMINAL_JOB_STATUSES = new Set(["done", "error", "cancelled", "interrupted"]);

/**
 * Non-stream state changes also carry activity facts: a job attachment means
 * the model is about to be waited on, and a terminal job sync ends the turn
 * even when the stream never delivered `run_completed` (cancel, force close).
 */
function activityEventForWorkbenchAction(action: WorkbenchAction): ActivityEvent | null {
  switch (action.type) {
    case "reset":
    case "hydrate":
      return { type: "reset" };
    case "prepare_job_attachment":
    case "job_created":
      return { type: "run_started" };
    case "job_state_synced":
      return TERMINAL_JOB_STATUSES.has(action.state.status)
        ? { type: "reset" }
        : { type: "run_started" };
    case "set_busy":
      return action.busy ? null : { type: "reset" };
    case "stream_event":
      return activityEventForStreamEvent(action.event);
    default:
      return null;
  }
}

function controlLabel(kind: ProductControlKind): string {
  return kind === "steer" ? "steer" : "follow-up";
}

const MAX_OLDER_HISTORY_ERROR_CHARS = 512;

/** The error state is a bounded, single line: never an unbounded dump. */
function boundErrorMessage(message: string): string {
  const compact = message.replace(/\s+/g, " ").trim();
  return compact.length <= MAX_OLDER_HISTORY_ERROR_CHARS
    ? compact
    : `${compact.slice(0, MAX_OLDER_HISTORY_ERROR_CHARS)}…`;
}

function olderHistoryFromResponse(
  transcript: ProductTranscriptResponse,
): OlderHistoryState {
  const hasMore = transcript.has_more === true;
  return {
    hasMore,
    cursor: hasMore ? (transcript.next_before_ordinal ?? null) : null,
    loading: false,
    error: null,
  };
}

/** The cursor for the next older page: below the oldest loaded run. */
function oldestLoadedOrdinal(
  segments: ProductTranscriptRunSegment[],
): number | null {
  let oldest: number | null = null;
  for (const segment of segments) {
    if (oldest === null || segment.binding.ordinal < oldest) {
      oldest = segment.binding.ordinal;
    }
  }
  return oldest;
}

function messageStatusLabel(status: ProductMessage["status"]): string {
  switch (status) {
    case "queued":
      return "Message queued for the next turn";
    case "intervention_requested":
      return "Intervention requested";
    case "applied_current_run":
      return "Message applied to the current run";
    case "claimed_successor":
      return "Starting queued message";
    case "needs_attention":
      return "Message needs attention";
    case "revoked":
      return "Message revoked";
  }
}

function isLikelyNetworkError(message: string): boolean {
  const normalized = message.toLowerCase();
  return (
    normalized.includes("fetch") ||
    normalized.includes("network") ||
    normalized.includes("connection") ||
    normalized.includes("timeout")
  );
}

function isAmbiguousJobStartError(error: unknown): boolean {
  if (error instanceof TypeError) {
    return true;
  }
  const message = describeError(error).toLowerCase();
  return [
    "fetch",
    "network",
    "connection",
    "load failed",
    "timeout",
    "timed out",
    "aborted",
  ].some((token) => message.includes(token));
}

const AMBIGUOUS_START_RECONCILIATION_DELAYS_MS = [
  0,
  100,
  200,
  400,
  800,
  1_000,
  1_000,
  1_000,
  1_000,
  1_000,
  1_000,
] as const;

function waitForReconciliationDelay(delayMs: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, delayMs));
}
