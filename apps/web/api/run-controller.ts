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
  approve(jobId: string, callId: string, decision: "approve" | "reject"): Promise<void>;
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
          dispatch({ type: "job_state_synced", state: jobState });
          if (isTerminalStatus(jobState.status)) {
            closeStream();
            options.onTerminal?.();
          }
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
    dispatch({
      type: "stream_event",
      event: payload,
      seq: parseEventSeq(message.lastEventId),
    });
    options.onStreamEvent?.(payload);
    if (payload.type === "run_completed") {
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
      dispatch({ type: "job_state_synced", state: jobState });
      if (isTerminalStatus(jobState.status)) {
        closeStream();
        options.onTerminal?.();
      } else {
        // The server replays buffered events, so opening the stream only after
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

    async approve(jobId, callId, decision) {
      const expectedGeneration = generation;
      const jobState = await submitApproval(jobId, callId, decision);
      assertCurrent(expectedGeneration);
      dispatch({ type: "approval_decision", callId, decision });
      dispatch({ type: "job_state_synced", state: jobState });
      if (isTerminalStatus(jobState.status)) {
        closeStream();
        options.onTerminal?.();
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
