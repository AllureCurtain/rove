import type { CreateJobRequest, JobStateResponse } from "../lib/rove-types";
import {
  cancelJob,
  createJob,
  fetchJobState,
  listProviderModels,
  openJobStream,
  submitApproval,
  submitInput,
  testProvider,
} from "../lib/rove-client";
import { STREAM_EVENT_NAMES, type StreamEvent } from "../lib/rove-types";
import type { WorkbenchAction } from "../lib/rove-state";

export function describeError(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}

export function parseEventSeq(value: string): number | undefined {
  if (!value) {
    return undefined;
  }
  const seq = Number(value);
  return Number.isSafeInteger(seq) && seq > 0 ? seq : undefined;
}

export function isTerminalStatus(status: string): boolean {
  return (
    status === "done" ||
    status === "error" ||
    status === "cancelled" ||
    status === "interrupted"
  );
}

export class RunControllerInactiveError extends Error {
  constructor() {
    super("run controller is no longer focused");
    this.name = "RunControllerInactiveError";
  }
}

export function isRunControllerInactive(error: unknown): boolean {
  return error instanceof RunControllerInactiveError;
}

export interface RunController {
  start(request: CreateJobRequest): Promise<{
    jobId: string;
    runId: string;
    resumedFromRunId: string | null;
  }>;
  attach(jobId: string, expectedRunId?: string | null): Promise<JobStateResponse>;
  cancel(jobId: string): Promise<void>;
  approve(jobId: string, callId: string, decision: "approve" | "reject", expectedRunId?: string): Promise<void>;
  answer(jobId: string, inputId: string, answer: string): Promise<void>;
  close(): void;
}

export function createRunController(
  dispatch: (action: WorkbenchAction) => void,
  options: {
    onTerminal?: () => void;
    onStreamEvent?: (event: StreamEvent) => void;
  } = {},
): RunController {
  let eventSource: EventSource | null = null;
  let generation = 0;
  let active = true;
  let resyncGeneration: number | null = null;
  let binding: { jobId: string; runId: string | null } | null = null;
  let terminal = false;
  let latestSeq = 0;
  const approvalRequests = new Set<string>();

  // Snapshots may race canonical events. Never roll a run back after a newer
  // event (especially run_completed), or project a reused job's different run.
  function syncSnapshot(state: JobStateResponse): boolean {
    if (binding && (state.job_id !== binding.jobId ||
        (binding.runId && state.run_id !== binding.runId))) {
      throw new Error("Approval snapshot does not match the observed job/run.");
    }
    if (state.event_count < latestSeq || (terminal && !isTerminalStatus(state.status))) {
      return false;
    }
    latestSeq = Math.max(latestSeq, state.event_count);
    dispatch({ type: "job_state_synced", state });
    if (isTerminalStatus(state.status) && !terminal) {
      terminal = true;
      closeStream();
      options.onTerminal?.();
    }
    return true;
  }

  function closeStream() {
    eventSource?.close();
    eventSource = null;
  }

  function isCurrent(expectedGeneration: number): boolean {
    return active && generation === expectedGeneration;
  }

  function assertCurrent(expectedGeneration: number) {
    if (!isCurrent(expectedGeneration)) {
      throw new RunControllerInactiveError();
    }
  }

  function beginObservation(): number {
    if (!active) {
      throw new RunControllerInactiveError();
    }
    generation += 1;
    terminal = false;
    latestSeq = 0;
    binding = null;
    closeStream();
    return generation;
  }

  function attachStream(jobId: string, expectedGeneration: number) {
    const source = openJobStream(jobId);
    eventSource = source;

    for (const name of STREAM_EVENT_NAMES) {
      source.addEventListener(name, ((event: Event) => {
        if (isCurrent(expectedGeneration)) {
          handleEvent(event, expectedGeneration);
        }
      }) as EventListener);
    }

    source.onerror = () => {
      if (
        !isCurrent(expectedGeneration) ||
        resyncGeneration === expectedGeneration
      ) {
        return;
      }
      resyncGeneration = expectedGeneration;
      dispatch({ type: "set_status", statusText: "Reconnecting event stream" });
      void fetchJobState(jobId)
        .then((jobState) => {
          if (!isCurrent(expectedGeneration)) {
            return;
          }
          syncSnapshot(jobState);
        })
        .catch((error) => {
          if (isCurrent(expectedGeneration)) {
            dispatch({ type: "set_error", error: describeError(error) });
          }
        })
        .finally(() => {
          if (resyncGeneration === expectedGeneration) {
            resyncGeneration = null;
          }
        });
    };
  }

  function handleEvent(event: Event, expectedGeneration: number) {
    if (!isCurrent(expectedGeneration)) {
      return;
    }
    const message = event as MessageEvent<string>;
    let payload: StreamEvent;
    try {
      payload = JSON.parse(message.data) as StreamEvent;
    } catch {
      dispatch({ type: "set_error", error: "Malformed stream event" });
      return;
    }
    latestSeq = Math.max(latestSeq, parseEventSeq(message.lastEventId) ?? 0);
    dispatch({
      type: "stream_event",
      event: payload,
      seq: parseEventSeq(message.lastEventId),
    });
    options.onStreamEvent?.(payload);
    if (payload.type === "run_completed") {
      terminal = true;
      closeStream();
      options.onTerminal?.();
    }
  }

  async function fetchExpectedJobState(
    jobId: string,
    expectedRunId: string | null | undefined,
    expectedGeneration: number,
  ): Promise<JobStateResponse> {
    for (const delayMs of ATTACH_RUN_RETRY_DELAYS_MS) {
      if (delayMs > 0) {
        await new Promise((resolve) => setTimeout(resolve, delayMs));
      }
      assertCurrent(expectedGeneration);
      const jobState = await fetchJobState(jobId);
      assertCurrent(expectedGeneration);
      if (!expectedRunId || jobState.run_id === expectedRunId) {
        return jobState;
      }
    }
    throw new Error(
      `job ${jobId} did not expose expected run ${expectedRunId} before attachment`,
    );
  }

  return {
    async start(request) {
      const expectedGeneration = beginObservation();
      const job = await createJob(request);
      assertCurrent(expectedGeneration);
      binding = { jobId: job.job_id, runId: job.run_id };
      dispatch({
        type: "job_created",
        jobId: job.job_id,
        runId: job.run_id,
        resumedFromRunId: job.resumed_from_run_id,
      });
      attachStream(job.job_id, expectedGeneration);
      return {
        jobId: job.job_id,
        runId: job.run_id,
        resumedFromRunId: job.resumed_from_run_id ?? null,
      };
    },

    async attach(jobId, expectedRunId) {
      const expectedGeneration = beginObservation();
      const jobState = await fetchExpectedJobState(
        jobId,
        expectedRunId,
        expectedGeneration,
      );
      binding = { jobId, runId: jobState.run_id ?? null };
      syncSnapshot(jobState);
      if (!terminal) {
        // exact run verification avoids attaching a reused job id to its prior run.
        attachStream(jobId, expectedGeneration);
      }
      return jobState;
    },

    async cancel(jobId) {
      const expectedGeneration = generation;
      dispatch({ type: "set_status", statusText: "Cancelling run" });
      const jobState = await cancelJob(jobId);
      assertCurrent(expectedGeneration);
      dispatch({ type: "job_state_synced", state: jobState });
      if (isTerminalStatus(jobState.status)) {
        closeStream();
        options.onTerminal?.();
      }
    },

    async approve(jobId, callId, decision, expectedRunId) {
      const expectedGeneration = generation;
      assertCurrent(expectedGeneration);
      if (!binding || binding.jobId !== jobId || !binding.runId ||
          (expectedRunId && binding.runId !== expectedRunId) || terminal) {
        throw new Error("Approval request is no longer current. Reload its session.");
      }
      const key = JSON.stringify([jobId, binding.runId, callId]);
      if (approvalRequests.has(key)) return;
      approvalRequests.add(key);
      try {
        const jobState = await approvalDeadline(submitApproval(jobId, callId, decision));
        assertCurrent(expectedGeneration);
        // The POST response is a snapshot, not evidence that this window's
        // decision executed the tool. Do not dispatch an optimistic decision.
        syncSnapshot(jobState);
      } catch (error) {
        assertCurrent(expectedGeneration);
        let reconciled = false;
        for (const delay of [0, 100, 300]) {
          if (delay) await new Promise((resolve) => setTimeout(resolve, delay));
          assertCurrent(expectedGeneration);
          try {
            const state = await approvalDeadline(fetchJobState(jobId));
            assertCurrent(expectedGeneration);
            reconciled = syncSnapshot(state);
            if (reconciled || terminal) break;
          } catch (readError) {
            if (isRunControllerInactive(readError)) throw readError;
          }
        }
        assertCurrent(expectedGeneration);
        throw new Error(`Approval outcome not confirmed: ${describeError(error)}. ${
          reconciled ? "Server state refreshed; the request may have been resolved elsewhere." :
            "Could not refresh authoritative state. Reload before deciding again."
        } No automatic resend.`);
      } finally {
        approvalRequests.delete(key);
      }
    },

    async answer(jobId, inputId, answer) {
      const expectedGeneration = generation;
      const jobState = await submitInput(jobId, inputId, answer);
      assertCurrent(expectedGeneration);
      dispatch({ type: "input_submitted", inputId });
      dispatch({ type: "job_state_synced", state: jobState });
      if (isTerminalStatus(jobState.status)) {
        closeStream();
        options.onTerminal?.();
      }
    },

    close() {
      active = false;
      generation += 1;
      closeStream();
    },
  };
}

function approvalDeadline<T>(request: Promise<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("Approval state request timed out")), 8_000);
    request.then(resolve, reject).finally(() => clearTimeout(timer));
  });
}

const ATTACH_RUN_RETRY_DELAYS_MS = [
  0,
  25,
  50,
  100,
  200,
  400,
  800,
  1_000,
  1_000,
  1_000,
] as const;

export { createJob, testProvider, listProviderModels, fetchJobState };
