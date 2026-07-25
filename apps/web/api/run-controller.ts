import type { CreateJobRequest } from "../lib/rove-types";
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

export interface RunController {
  start(request: CreateJobRequest): Promise<{
    jobId: string;
    runId: string;
    resumedFromRunId: string | null;
  }>;
  cancel(jobId: string): Promise<void>;
  approve(jobId: string, callId: string, decision: "approve" | "reject"): Promise<void>;
  answer(jobId: string, inputId: string, answer: string): Promise<void>;
  close(): void;
}

export function createRunController(
  dispatch: (action: WorkbenchAction) => void,
  options: {
    onTerminal?: () => void;
  } = {},
): RunController {
  let eventSource: EventSource | null = null;

  function closeStream() {
    eventSource?.close();
    eventSource = null;
  }

  function attachStream(jobId: string) {
    closeStream();
    const source = openJobStream(jobId);
    eventSource = source;

    for (const name of STREAM_EVENT_NAMES) {
      source.addEventListener(name, handleEvent as EventListener);
    }

    source.onerror = () => {
      dispatch({ type: "set_status", statusText: "Reconnecting event stream" });
      void fetchJobState(jobId)
        .then((jobState) => {
          dispatch({ type: "job_state_synced", state: jobState });
          if (isTerminalStatus(jobState.status)) {
            closeStream();
            options.onTerminal?.();
          }
        })
        .catch((error) => {
          dispatch({ type: "set_error", error: describeError(error) });
        });
    };
  }

  function handleEvent(event: Event) {
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
    if (payload.type === "run_completed") {
      closeStream();
      options.onTerminal?.();
    }
  }

  return {
    async start(request) {
      const job = await createJob(request);
      dispatch({
        type: "job_created",
        jobId: job.job_id,
        runId: job.run_id,
        resumedFromRunId: job.resumed_from_run_id,
      });
      attachStream(job.job_id);
      return {
        jobId: job.job_id,
        runId: job.run_id,
        resumedFromRunId: job.resumed_from_run_id ?? null,
      };
    },

    async cancel(jobId) {
      dispatch({ type: "set_status", statusText: "Cancelling run" });
      try {
        const jobState = await cancelJob(jobId);
        dispatch({ type: "job_state_synced", state: jobState });
        if (isTerminalStatus(jobState.status)) {
          options.onTerminal?.();
        }
      } finally {
        closeStream();
      }
    },

    async approve(jobId, callId, decision) {
      const jobState = await submitApproval(jobId, callId, decision);
      dispatch({ type: "approval_decision", callId, decision });
      dispatch({ type: "job_state_synced", state: jobState });
      if (isTerminalStatus(jobState.status)) {
        closeStream();
        options.onTerminal?.();
      }
    },

    async answer(jobId, inputId, answer) {
      const jobState = await submitInput(jobId, inputId, answer);
      dispatch({ type: "input_submitted", inputId });
      dispatch({ type: "job_state_synced", state: jobState });
      if (isTerminalStatus(jobState.status)) {
        closeStream();
        options.onTerminal?.();
      }
    },

    close() {
      closeStream();
    },
  };
}

export { createJob, testProvider, listProviderModels, fetchJobState };
