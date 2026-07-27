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
} from "../lib/rove-state";
import type { ProductApiClient } from "../product/product-client";
import {
  findSession,
  findWorkspace,
  updateSession,
  type ProductCatalog,
} from "./product-catalog";
import type {
  ActiveProviderSelection,
  ProviderProfileRecord,
  SessionRecord,
} from "./product-types";
import {
  projectProductTranscript,
  type TranscriptRestoreState,
} from "./transcript-projection";
import { buildTurnJobRequest, isHardResumeError } from "./turn-request";
import { hasAdvancedRuntimeBinding } from "./server-product-state";

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
}) {
  const [restoreState, setRestoreState] = useState<TranscriptRestoreState>({
    status: "idle",
  });
  const [approvalBusy, setApprovalBusy] = useState<string | null>(null);
  const [inputBusy, setInputBusy] = useState<string | null>(null);
  const [runState, dispatch] = useReducer(
    workbenchReducer,
    undefined,
    createWorkbenchState,
  );
  const controllerRef = useRef<ReturnType<typeof createRunController> | null>(null);
  const focusedSessionRef = useRef<string | null>(null);
  const restoredSessionRef = useRef<string | null>(null);
  const observedJobRef = useRef<string | null>(null);
  const transcriptGenerationRef = useRef(0);
  const refreshStatusesRef = useRef(refreshSessionStatuses);

  useEffect(() => {
    refreshStatusesRef.current = refreshSessionStatuses;
  }, [refreshSessionStatuses]);

  const closeFocusedObservation = useCallback(() => {
    controllerRef.current?.close();
    controllerRef.current = null;
    observedJobRef.current = null;
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
          observedJobRef.current = null;
          markSession(sessionId, { status: "idle", hasDurableTurn: true });
          void refreshStatusesRef.current();
        },
      });
      controllerRef.current = controller;
      return controller;
    },
    [closeFocusedObservation, markSession],
  );

  const attachFocusedJob = useCallback(
    (sessionId: string, jobId: string) => {
      if (
        focusedSessionRef.current !== sessionId ||
        observedJobRef.current === jobId
      ) {
        return;
      }
      const controller = installFocusedController(sessionId);
      observedJobRef.current = jobId;
      dispatch({ type: "prepare_job_attachment", jobId });
      void controller.attach(jobId).catch((error) => {
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
      dispatch({ type: "reset" });
      setRestoreState({ status: "loading", sessionId });

      try {
        const transcript = await productClient.getTranscript(sessionId);
        if (
          transcriptGenerationRef.current !== generation ||
          focusedSessionRef.current !== sessionId
        ) {
          return;
        }
        if (transcript.workspace_id !== workspaceId) {
          throw new Error("Transcript workspace does not match the session route.");
        }
        const projected = projectProductTranscript(transcript);
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

        const session = findSession(catalogRef.current, sessionId);
        const liveJobId =
          projected.busy && projected.activeJobId
            ? projected.activeJobId
            : session &&
                (session.status === "running" || session.status === "needs_attention")
              ? session.activeJobId ?? null
              : null;
        if (liveJobId) {
          attachFocusedJob(sessionId, liveJobId);
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
    [attachFocusedJob, catalogRef, closeFocusedObservation, productClient],
  );

  const focusSession = useCallback(
    (workspaceId: string, sessionId: string) => {
      if (restoredSessionRef.current !== sessionId) {
        void restoreSession(workspaceId, sessionId);
      }
    },
    [restoreSession],
  );

  const prepareSession = useCallback(
    (sessionId: string) => {
      ++transcriptGenerationRef.current;
      closeFocusedObservation();
      focusedSessionRef.current = sessionId;
      restoredSessionRef.current = null;
      dispatch({ type: "reset" });
      setRestoreState({ status: "loading", sessionId });
    },
    [closeFocusedObservation],
  );

  const leaveSession = useCallback(() => {
    ++transcriptGenerationRef.current;
    closeFocusedObservation();
    focusedSessionRef.current = null;
    restoredSessionRef.current = null;
    dispatch({ type: "reset" });
    setRestoreState({ status: "idle" });
  }, [closeFocusedObservation]);

  const retryRestore = useCallback(
    (workspaceId: string, sessionId: string) => {
      restoredSessionRef.current = null;
      void restoreSession(workspaceId, sessionId);
    },
    [restoreSession],
  );

  const reconcileCreatedTurn = useCallback(
    async (
      workspaceId: string,
      previousSession: SessionRecord,
      controller: ReturnType<typeof createRunController>,
    ) => {
      dispatch({ type: "set_status", statusText: "Reconciling durable session" });
      for (const delayMs of AMBIGUOUS_START_RECONCILIATION_DELAYS_MS) {
        if (delayMs > 0) {
          await waitForReconciliationDelay(delayMs);
        }
        if (
          controllerRef.current !== controller ||
          focusedSessionRef.current !== previousSession.id
        ) {
          return false;
        }

        const refreshed = await refreshStatusesRef.current();
        if (
          !refreshed ||
          controllerRef.current !== controller ||
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

        observedJobRef.current = currentSession.activeJobId;
        dispatch({
          type: "prepare_job_attachment",
          jobId: currentSession.activeJobId,
        });
        try {
          await controller.attach(currentSession.activeJobId);
        } catch (error) {
          if (
            isRunControllerInactive(error) ||
            focusedSessionRef.current !== previousSession.id
          ) {
            return true;
          }
          restoredSessionRef.current = null;
          await restoreSession(workspaceId, previousSession.id);
        }
        if (focusedSessionRef.current === previousSession.id) {
          setConnection("ok");
        }
        return true;
      }
      return false;
    },
    [catalogRef, restoreSession, setConnection],
  );

  const send = useCallback(
    async (message: string) => {
      const workspace = findWorkspace(
        catalogRef.current,
        catalogRef.current.active.workspaceId,
      );
      const session = findSession(
        catalogRef.current,
        catalogRef.current.active.sessionId,
      );
      if (!workspace || !session) {
        dispatch({ type: "set_error", error: "Open a workspace and session first." });
        return;
      }

      let request;
      try {
        request = buildTurnJobRequest({
          message,
          workspace,
          session,
          selection,
          profiles,
        });
      } catch (error) {
        dispatch({ type: "set_error", error: describeError(error) });
        return;
      }

      const controller = installFocusedController(session.id);
      dispatch({ type: "prepare_turn", preserveTools: true });
      dispatch({ type: "append_user_message", content: message });
      dispatch({ type: "set_status", statusText: "Submitting job" });
      const title = session.title === "New session" ? truncateTitle(message) : session.title;
      markSession(session.id, {
        status: "running",
        title,
        activeJobId: null,
        activeRunId: null,
        resumedFromRunId: null,
      });
      if (session.title === "New session") {
        void updateSessionTitle(session.id, title);
      }

      try {
        const started = await controller.start(request);
        if (
          controllerRef.current !== controller ||
          focusedSessionRef.current !== session.id
        ) {
          return;
        }
        observedJobRef.current = started.jobId;
        markSession(session.id, {
          activeJobId: started.jobId,
          activeRunId: started.runId,
          resumedFromRunId: started.resumedFromRunId,
          hasDurableTurn: true,
          status: "running",
        });
        setConnection("ok");
      } catch (error) {
        if (
          isRunControllerInactive(error) ||
          focusedSessionRef.current !== session.id
        ) {
          return;
        }
        if (isAmbiguousJobStartError(error)) {
          dispatch({
            type: "set_status",
            statusText: "Confirming whether the server started this turn",
          });
          if (await reconcileCreatedTurn(workspace.id, session, controller)) {
            return;
          }
          if (
            controllerRef.current !== controller ||
            focusedSessionRef.current !== session.id
          ) {
            return;
          }
          setConnection("error");
          markSession(session.id, { status: "needs_attention" });
          if (focusedSessionRef.current === session.id) {
            restoredSessionRef.current = null;
            await restoreSession(workspace.id, session.id);
            if (focusedSessionRef.current === session.id) {
              setConnection("error");
              dispatch({
                type: "set_error",
                error:
                  "The server may have accepted this turn, but its durable binding is not visible yet. Reload before sending another message.",
              });
            }
          }
          return;
        }
        const messageText = describeError(error);
        const hard = session.hasDurableTurn || isHardResumeError(messageText);
        dispatch({
          type: "set_error",
          error: hard ? `Exact session resume failed: ${messageText}.` : messageText,
        });
        markSession(session.id, { status: "error" });
        if (
          messageText.toLowerCase().includes("fetch") ||
          messageText.toLowerCase().includes("network")
        ) {
          setConnection("error");
        }
      }
    },
    [
      catalogRef,
      installFocusedController,
      markSession,
      profiles,
      reconcileCreatedTurn,
      selection,
      setConnection,
      restoreSession,
      updateSessionTitle,
    ],
  );

  const cancel = useCallback(async () => {
    if (!runState.activeJobId) {
      return;
    }
    try {
      await controllerRef.current?.cancel(runState.activeJobId);
      if (activeSession) {
        markSession(activeSession.id, { status: "idle" });
      }
    } catch (error) {
      if (!isRunControllerInactive(error)) {
        dispatch({ type: "set_error", error: describeError(error) });
      }
    }
  }, [activeSession, markSession, runState.activeJobId]);

  const approve = useCallback(
    async (tool: ToolCallView, decision: "approve" | "reject") => {
      if (!runState.activeJobId || !tool.pendingApproval) {
        return;
      }
      setApprovalBusy(tool.id);
      try {
        await controllerRef.current?.approve(runState.activeJobId, tool.id, decision);
        if (activeSession) {
          markSession(activeSession.id, {
            status: decision === "reject" ? "idle" : "running",
          });
        }
      } catch (error) {
        if (!isRunControllerInactive(error)) {
          dispatch({ type: "set_error", error: describeError(error) });
          if (activeSession) {
            markSession(activeSession.id, { status: "error" });
          }
        }
      } finally {
        setApprovalBusy(null);
      }
    },
    [activeSession, markSession, runState.activeJobId],
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
      !activeSession.activeJobId
    ) {
      return;
    }
    attachFocusedJob(activeSession.id, activeSession.activeJobId);
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
    restoreState,
    approvalBusy,
    inputBusy,
    focusSession,
    prepareSession,
    leaveSession,
    retryRestore,
    send,
    cancel,
    approve,
    answer,
  };
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

function truncateTitle(message: string): string {
  const compact = message.replace(/\s+/g, " ").trim();
  return compact.length <= 42 ? compact : `${compact.slice(0, 42)}...`;
}
