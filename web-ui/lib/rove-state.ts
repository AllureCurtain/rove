import type {
  PlanStep,
  StreamEvent,
  TaskPlan,
  ToolError,
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
  lastSignal: string;
  busy: boolean;
  error: string | null;
  messages: ChatMessage[];
  plan: TaskPlan | null;
  tools: ToolCallView[];
  trace: TraceEntry[];
}

export type WorkbenchAction =
  | { type: "reset" }
  | { type: "append_user_message"; content: string }
  | { type: "job_created"; jobId: string; runId: string }
  | { type: "set_busy"; busy: boolean }
  | { type: "set_status"; statusText: string }
  | { type: "set_error"; error: string | null }
  | { type: "stream_event"; event: StreamEvent };

export function createWorkbenchState(): WorkbenchState {
  return {
    activeJobId: null,
    activeRunId: null,
    statusText: "No active run",
    eventCount: 0,
    lastSignal: "Idle",
    busy: false,
    error: null,
    messages: [],
    plan: null,
    tools: [],
    trace: [],
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
    case "stream_event":
      return applyStreamEvent(state, action.event);
  }
}

function applyStreamEvent(state: WorkbenchState, event: StreamEvent): WorkbenchState {
  const next = {
    ...state,
    eventCount: state.eventCount + 1,
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
