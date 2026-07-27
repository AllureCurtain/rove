import {
  createWorkbenchState,
  workbenchReducer,
  type ChatMessage,
  type WorkbenchState,
} from "../lib/rove-state";
import type {
  ProductStreamEvent,
  ProductTranscriptPartialReason,
  ProductTranscriptResponse,
  ProductTranscriptRunSegment,
} from "../product/product-api-types";
import type { StreamEvent } from "../lib/rove-types";

export type TranscriptRestoreState =
  | { status: "idle" }
  | { status: "loading"; sessionId: string }
  | { status: "complete"; sessionId: string }
  | {
      status: "partial";
      sessionId: string;
      reasons: ProductTranscriptPartialReason[];
    }
  | { status: "error"; sessionId: string; error: string };

export function projectProductTranscript(
  transcript: ProductTranscriptResponse,
): WorkbenchState {
  let state = createWorkbenchState();

  for (const [index, segment] of transcript.segments.entries()) {
    if (index > 0) {
      state = workbenchReducer(state, {
        type: "prepare_turn",
        preserveTools: true,
      });
    }
    state = workbenchReducer(state, {
      type: "job_created",
      jobId: segment.binding.runtime_job_id,
      runId: segment.binding.runtime_run_id,
      resumedFromRunId: segment.binding.resumed_from_run_id ?? null,
    });
    for (const stored of segment.events) {
      state = workbenchReducer(state, {
        type: "stream_event",
        event: toWorkbenchStreamEvent(stored.event),
        seq: stored.seq,
      });
    }
    state = settleRestoredSegment(state, segment);
  }

  return {
    ...state,
    error: null,
    lastSignal: transcript.segments.length > 0 ? "Transcript restored" : "Idle",
  };
}

export function toWorkbenchStreamEvent(event: ProductStreamEvent): StreamEvent {
  switch (event.type) {
    case "tool_call_completed":
      return {
        type: event.type,
        call_id: event.call_id,
        result: {
          call_id: event.result.call_id,
          output: event.result.output,
          mutations: event.result.mutations,
        },
      };
    case "tool_call_failed":
      return {
        type: event.type,
        call_id: event.call_id,
        error: event.error,
      };
    case "prompt_built":
      return {
        type: event.type,
        metadata: {
          ...event.metadata,
          prompt_cache_key:
            event.metadata.prompt_cache_key ?? event.metadata.prompt_hash,
        },
      };
    default:
      return event;
  }
}

export function describeTranscriptPartialReason(
  reason: ProductTranscriptPartialReason,
): string {
  const location = reason.run_ordinal
    ? `run ${reason.run_ordinal}${reason.run_id ? ` (${shortId(reason.run_id)})` : ""}`
    : reason.run_id
      ? `run ${shortId(reason.run_id)}`
      : "session history";
  const detail = PARTIAL_REASON_LABELS[reason.code];
  const sequence =
    reason.expected_seq !== undefined || reason.observed_seq !== undefined
      ? ` Expected event ${reason.expected_seq ?? "?"}, observed ${reason.observed_seq ?? "?"}.`
      : "";
  return `${location}: ${detail}.${sequence}`;
}

const PARTIAL_REASON_LABELS: Record<ProductTranscriptPartialReason["code"], string> = {
  missing_run_mapping: "a durable run mapping is missing",
  runtime_run_missing: "the mapped runtime run is unavailable",
  runtime_state_unavailable: "the runtime state store could not be read",
  runtime_identity_mismatch: "the durable runtime identity does not match",
  missing_event_range: "part of the canonical event range is missing",
  corrupt_event: "a canonical event could not be decoded",
  corrupt_artifact: "a durable runtime artifact is corrupt",
  cleaned_history: "part of the history has been cleaned up",
  response_limit_reached: "the transcript response reached its safety limit",
};

function settleRestoredSegment(
  state: WorkbenchState,
  segment: ProductTranscriptRunSegment,
): WorkbenchState {
  const busy = segment.run_status === "init" || segment.run_status === "running";
  const fallbackMessages = segment.fallback?.summary
      ? [
          ...state.messages,
          {
            id: `fallback-${segment.binding.runtime_run_id}`,
            role: "assistant" as const,
            content: `Report summary: ${segment.fallback.summary}`,
            status: "final" as const,
          },
        ]
      : state.messages;
  const tools = busy
    ? state.tools
    : state.tools.map((tool) => ({
        ...tool,
        status: tool.status === "waiting" ? ("error" as const) : tool.status,
        pendingApproval: undefined,
      }));

  return {
    ...state,
    activeJobId: segment.binding.runtime_job_id,
    activeRunId: segment.binding.runtime_run_id,
    resumedFromRunId: segment.binding.resumed_from_run_id ?? null,
    eventCount: segment.observed_through_seq,
    busy,
    statusText: restoredStatusText(segment.run_status),
    tools,
    pendingInputs: busy ? state.pendingInputs : [],
    messages: busy
      ? fallbackMessages
      : finalizeRestoredMessages(fallbackMessages),
  };
}

function finalizeRestoredMessages(messages: ChatMessage[]): ChatMessage[] {
  return messages.map((message) =>
    message.status === "streaming" ? { ...message, status: "final" } : message,
  );
}

function restoredStatusText(status: ProductTranscriptRunSegment["run_status"]): string {
  switch (status) {
    case "init":
      return "Job queued";
    case "running":
      return "Streaming run events";
    case "done":
      return "Run completed";
    case "error":
      return "Run failed";
    case "cancelled":
      return "Run cancelled";
    case "interrupted":
      return "Run interrupted";
  }
}

function shortId(value: string): string {
  return value.length <= 12 ? value : value.slice(0, 12);
}
