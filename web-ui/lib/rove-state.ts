import type {
  ApprovalDecision,
  PendingInput,
  PlanStep,
  PendingApproval,
  StreamEvent,
  TaskPlan,
  ToolError,
  JobStateResponse,
} from "./rove-types";

export interface ChatMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
  status: "streaming" | "final";
}

export interface ToolCallView {
  id: string;
  name: string;
  status: "running" | "waiting" | "done" | "error";
  details: string;
  reason?: string;
  pendingApproval?: PendingApproval;
}

export interface TraceEntry {
  id: string;
  label: string;
  detail: string;
}

export interface WorkbenchState {
  activeJobId: string | null;
  activeRunId: string | null;
  statusText: string;
  eventCount: number;
  seenEventSeqs: number[];
  lastSignal: string;
  busy: boolean;
  error: string | null;
  messages: ChatMessage[];
  plan: TaskPlan | null;
  tools: ToolCallView[];
  trace: TraceEntry[];
  pendingInputs: PendingInput[];
}

export type WorkbenchAction =
  | { type: "reset" }
  | { type: "append_user_message"; content: string }
  | { type: "job_created"; jobId: string; runId: string }
  | { type: "set_busy"; busy: boolean }
  | { type: "set_status"; statusText: string }
  | { type: "set_error"; error: string | null }
  | { type: "approval_decision"; callId: string; decision: ApprovalDecision }
  | { type: "input_submitted"; inputId: string }
  | { type: "job_state_synced"; state: JobStateResponse }
  | { type: "stream_event"; event: StreamEvent; seq?: number };

export function createWorkbenchState(): WorkbenchState {
  return {
    activeJobId: null,
    activeRunId: null,
    statusText: "No active run",
    eventCount: 0,
    seenEventSeqs: [],
    lastSignal: "Idle",
    busy: false,
    error: null,
    messages: [],
    plan: null,
    tools: [],
    trace: [],
    pendingInputs: [],
  };
}

export function workbenchReducer(
  state: WorkbenchState,
  action: WorkbenchAction,
): WorkbenchState {
  switch (action.type) {
    case "reset":
      return createWorkbenchState();
    case "append_user_message":
      return {
        ...state,
        messages: [
          ...state.messages,
          {
            id: crypto.randomUUID(),
            role: "user",
            content: action.content,
            status: "final",
          },
        ],
      };
    case "job_created":
      return {
        ...state,
        activeJobId: action.jobId,
        activeRunId: action.runId,
        busy: true,
        statusText: "Streaming run events",
        lastSignal: "Job created",
        error: null,
      };
    case "set_busy":
      return {
        ...state,
        busy: action.busy,
      };
    case "set_status":
      return {
        ...state,
        statusText: action.statusText,
      };
    case "set_error":
      return {
        ...state,
        error: action.error,
        statusText: action.error ? "Run interrupted" : state.statusText,
        lastSignal: action.error ? "Error" : state.lastSignal,
      };
    case "approval_decision":
      return {
        ...state,
        tools: state.tools.map((tool) =>
          tool.id === action.callId
            ? {
                ...tool,
                status: action.decision === "approve" ? "running" : "error",
                details:
                  action.decision === "approve" ? "Approval submitted" : "Rejected by user",
                pendingApproval: undefined,
              }
            : tool,
        ),
      };
    case "input_submitted":
      return {
        ...state,
        pendingInputs: state.pendingInputs.filter(
          (input) => input.input_id !== action.inputId,
        ),
      };
    case "job_state_synced":
      return applyJobState(state, action.state);
    case "stream_event":
      return applyStreamEvent(state, action.event, action.seq);
  }
}

function applyJobState(
  state: WorkbenchState,
  jobState: JobStateResponse,
): WorkbenchState {
  const hydrated = jobState.events.reduce(
    (current, stored) => applyStreamEvent(current, stored.event, stored.seq),
    state,
  );
  const busy = jobState.status === "init" || jobState.status === "running";
  const terminalDetail = statusDetail(jobState.status);

  return {
    ...hydrated,
    activeJobId: jobState.job_id,
    activeRunId: jobState.run_id,
    eventCount: jobState.event_count,
    busy,
    error: jobState.status === "error" ? (state.error ?? "Run failed") : state.error,
    statusText: statusText(jobState.status),
    lastSignal: "Job state synced",
    pendingInputs: jobState.pending_inputs,
    tools: syncPendingApprovals(
      hydrated.tools,
      jobState.pending_approvals,
      busy ? undefined : terminalDetail,
    ),
  };
}

function syncPendingApprovals(
  tools: ToolCallView[],
  pendingApprovals: PendingApproval[],
  terminalDetail?: string,
): ToolCallView[] {
  const pendingById = new Map(
    pendingApprovals.map((approval) => [approval.call_id, approval]),
  );
  const existingIds = new Set(tools.map((tool) => tool.id));
  const synced = tools.map((tool) => {
    const pending = pendingById.get(tool.id);
    if (pending) {
      return toolFromPendingApproval(pending, tool);
    }
    if (tool.pendingApproval && terminalDetail) {
      return {
        ...tool,
        status: "error" as const,
        details: terminalDetail,
        pendingApproval: undefined,
      };
    }
    if (tool.pendingApproval) {
      return {
        ...tool,
        status: "running" as const,
        details: "Approval state synced",
        pendingApproval: undefined,
      };
    }
    return tool;
  });
  const inserted = pendingApprovals
    .filter((approval) => !existingIds.has(approval.call_id))
    .map((approval) => toolFromPendingApproval(approval));
  return [...inserted, ...synced];
}

function toolFromPendingApproval(
  pendingApproval: PendingApproval,
  existing?: ToolCallView,
): ToolCallView {
  return {
    ...existing,
    id: pendingApproval.call_id,
    name: pendingApproval.name,
    status: "waiting",
    details: pendingApproval.reason,
    reason: pendingApproval.reason,
    pendingApproval,
  };
}

function statusText(status: JobStateResponse["status"]): string {
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

function statusDetail(status: JobStateResponse["status"]): string {
  switch (status) {
    case "init":
      return "Job queued";
    case "running":
      return "Run still active";
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

function applyStreamEvent(
  state: WorkbenchState,
  event: StreamEvent,
  seq?: number,
): WorkbenchState {
  if (seq !== undefined && state.seenEventSeqs.includes(seq)) {
    return state;
  }

  const next = {
    ...state,
    eventCount: seq === undefined ? state.eventCount + 1 : Math.max(state.eventCount, seq),
    seenEventSeqs:
      seq === undefined ? state.seenEventSeqs : [...state.seenEventSeqs, seq],
    lastSignal: humanizeEventName(event.type),
  };

  switch (event.type) {
    case "run_started":
      return {
        ...next,
        activeJobId: event.job_id,
        activeRunId: event.run_id,
        busy: true,
        error: null,
        statusText: "Run started",
        messages: ensureUserMessage(state.messages, event.user_message),
        trace: prependTrace(state.trace, event.type, `${event.user_message}`),
      };
    case "llm_chunk":
      return {
        ...next,
        messages: appendAssistantDelta(state.messages, event.delta),
      };
    case "llm_message":
      return {
        ...next,
        messages: finalizeAssistantMessage(state.messages, event.full),
      };
    case "tool_call_started":
      return {
        ...next,
        tools: upsertTool(state.tools, {
          id: event.call_id,
          name: event.name,
          status: "running",
          details: formatValue(event.args),
        }),
        trace: prependTrace(
          state.trace,
          event.type,
          `${event.name} ${formatValue(event.args)}`,
        ),
      };
    case "tool_call_approval_needed":
      return {
        ...next,
        tools: upsertTool(state.tools, {
          id: event.call_id,
          name: event.name,
          status: "waiting",
          details: event.reason,
          reason: event.reason,
          pendingApproval: {
            call_id: event.call_id,
            name: event.name,
            args: event.args,
            reason: event.reason,
          },
        }),
        trace: prependTrace(
          state.trace,
          event.type,
          `${event.name} ${event.reason}`,
        ),
      };
    case "tool_call_completed":
      return {
        ...next,
        tools: upsertTool(state.tools, {
          id: event.call_id,
          name: findToolName(state.tools, event.result.call_id),
          status: "done",
          details: event.result.output,
        }),
        trace: prependTrace(
          state.trace,
          event.type,
          truncate(event.result.output, 220),
        ),
      };
    case "tool_call_failed":
      return {
        ...next,
        tools: upsertTool(state.tools, {
          id: event.call_id,
          name: findToolName(state.tools, event.call_id),
          status: "error",
          details: formatToolError(event.error),
        }),
        trace: prependTrace(
          state.trace,
          event.type,
          formatToolError(event.error),
        ),
      };
    case "input_needed":
      return {
        ...next,
        pendingInputs: [
          ...state.pendingInputs.filter((input) => input.input_id !== event.input_id),
          { input_id: event.input_id, prompt: event.prompt },
        ],
        trace: prependTrace(state.trace, event.type, event.prompt),
      };
    case "plan_created":
      return {
        ...next,
        plan: event.plan,
        trace: prependTrace(state.trace, event.type, `${event.plan.steps.length} steps`),
      };
    case "plan_step_started":
      return {
        ...next,
        plan: updatePlan(state.plan, event.index, event.step, false),
        trace: prependTrace(state.trace, event.type, event.step.title),
      };
    case "plan_step_completed":
      return {
        ...next,
        plan: updatePlan(state.plan, event.index, event.step, true),
        trace: prependTrace(state.trace, event.type, event.step.title),
      };
    case "plan_step_failed":
      return {
        ...next,
        plan: updatePlan(state.plan, event.index, event.step, false),
        trace: prependTrace(
          state.trace,
          event.type,
          `${event.step.title}: ${event.reason}`,
        ),
      };
    case "run_completed":
      return {
        ...next,
        busy: false,
        statusText: `Run completed: ${event.reason}`,
        lastSignal: `Run completed`,
        messages: finalizeRunMessage(state.messages, event.output),
        trace: prependTrace(
          state.trace,
          event.type,
          event.output ? truncate(event.output, 220) : event.reason,
        ),
      };
  }
}

function ensureUserMessage(messages: ChatMessage[], content: string): ChatMessage[] {
  if (
    messages.some(
      (message) =>
        message.role === "user" &&
        message.content === content &&
        message.status === "final",
    )
  ) {
    return messages;
  }
  return [
    ...messages,
    {
      id: crypto.randomUUID(),
      role: "user",
      content,
      status: "final",
    },
  ];
}

function appendAssistantDelta(messages: ChatMessage[], delta: string): ChatMessage[] {
  if (delta.length === 0) {
    return messages;
  }
  const last = messages[messages.length - 1];
  if (last?.role === "assistant" && last.status === "streaming") {
    return [
      ...messages.slice(0, -1),
      {
        ...last,
        content: last.content + delta,
      },
    ];
  }
  return [
    ...messages,
    {
      id: crypto.randomUUID(),
      role: "assistant",
      content: delta,
      status: "streaming",
    },
  ];
}

function finalizeAssistantMessage(
  messages: ChatMessage[],
  full: string,
): ChatMessage[] {
  const last = messages[messages.length - 1];
  if (last?.role === "assistant" && last.status === "streaming") {
    return [
      ...messages.slice(0, -1),
      {
        ...last,
        content: full,
        status: "final",
      },
    ];
  }
  return [
    ...messages,
    {
      id: crypto.randomUUID(),
      role: "assistant",
      content: full,
      status: "final",
    },
  ];
}

function finalizeRunMessage(
  messages: ChatMessage[],
  output: string | null | undefined,
): ChatMessage[] {
  if (!output) {
    return messages;
  }
  const last = messages[messages.length - 1];
  if (last?.role === "assistant") {
    return [
      ...messages.slice(0, -1),
      {
        ...last,
        content: output,
        status: "final",
      },
    ];
  }
  return [
    ...messages,
    {
      id: crypto.randomUUID(),
      role: "assistant",
      content: output,
      status: "final",
    },
  ];
}

function prependTrace(trace: TraceEntry[], label: string, detail: string): TraceEntry[] {
  return [
    {
      id: crypto.randomUUID(),
      label,
      detail,
    },
    ...trace,
  ];
}

function upsertTool(
  tools: ToolCallView[],
  next: ToolCallView,
): ToolCallView[] {
  const index = tools.findIndex((tool) => tool.id === next.id);
  if (index === -1) {
    return [next, ...tools];
  }
  const current = tools[index];
  return [
    ...tools.slice(0, index),
    {
      ...current,
      ...next,
      name: next.name || current.name,
    },
    ...tools.slice(index + 1),
  ];
}

function updatePlan(
  plan: TaskPlan | null,
  index: number,
  step: PlanStep,
  completed: boolean,
): TaskPlan | null {
  if (!plan) {
    return null;
  }
  const steps = plan.steps.map((current, currentIndex) =>
    currentIndex === index ? { ...step, done: completed } : current,
  );
  const currentStep = completed
    ? Math.min(index + 1, steps.length)
    : Math.min(index, steps.length);
  return {
    ...plan,
    steps,
    current_step: currentStep,
  };
}

function findToolName(tools: ToolCallView[], id: string): string {
  return tools.find((tool) => tool.id === id)?.name ?? "tool";
}

function formatToolError(error: ToolError): string {
  return formatValue(error);
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

function truncate(value: string, max: number): string {
  return value.length <= max ? value : `${value.slice(0, max)}…`;
}

function humanizeEventName(value: string): string {
  return value
    .replaceAll("_", " ")
    .replace(/\b\w/g, (match) => match.toUpperCase());
}
