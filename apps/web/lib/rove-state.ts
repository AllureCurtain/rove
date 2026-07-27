import type {
  ApprovalDecision,
  PendingInput,
  PlanDecisionRecord,
  PlanRevision,
  PlanStep,
  PendingApproval,
  StepRecord,
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
  resumedFromRunId: string | null;
  statusText: string;
  eventCount: number;
  seenEventSeqs: number[];
  lastSignal: string;
  busy: boolean;
  error: string | null;
  messages: ChatMessage[];
  /** Optimistic user bubble awaiting its canonical run_started identity. */
  pendingUserMessageId: string | null;
  /** Run identities whose canonical user bubble has already been projected. */
  seenUserMessageRunIds: string[];
  plan: TaskPlan | null;
  planDecisions: PlanDecisionRecord[];
  planRevisions: PlanRevision[];
  stepRecords: StepRecord[];
  /** Terminal tool cards retained from earlier product-session runs. */
  historicalTools: ToolCallView[];
  tools: ToolCallView[];
  trace: TraceEntry[];
  pendingInputs: PendingInput[];
}

export type WorkbenchAction =
  | { type: "reset" }
  | { type: "hydrate"; state: WorkbenchState }
  /** Clear run-scoped view state while optionally retaining terminal product tool history. */
  | { type: "prepare_turn"; preserveTools?: boolean }
  /** Reset stale run identity before following a different durable job. */
  | { type: "prepare_job_attachment"; jobId: string }
  | { type: "append_user_message"; content: string }
  | {
      type: "job_created";
      jobId: string;
      runId: string;
      resumedFromRunId?: string | null;
    }
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
    resumedFromRunId: null,
    statusText: "No active run",
    eventCount: 0,
    seenEventSeqs: [],
    lastSignal: "Idle",
    busy: false,
    error: null,
    messages: [],
    pendingUserMessageId: null,
    seenUserMessageRunIds: [],
    plan: null,
    planDecisions: [],
    planRevisions: [],
    stepRecords: [],
    historicalTools: [],
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
    case "hydrate":
      return action.state;
    case "prepare_turn":
      return prepareRunScopedState(
        state,
        action.preserveTools ?? false,
        "Preparing turn",
      );
    case "prepare_job_attachment":
      if (!state.activeJobId || state.activeJobId === action.jobId) {
        return {
          ...state,
          activeJobId: action.jobId,
          statusText: "Connecting to active run",
          lastSignal: "Connecting to active run",
          busy: true,
          error: null,
        };
      }
      return {
        ...prepareRunScopedState(state, true, "Connecting to active run"),
        activeJobId: action.jobId,
        busy: true,
      };
    case "append_user_message": {
      const userMessageId = crypto.randomUUID();
      return {
        ...state,
        messages: [
          ...state.messages,
          {
            id: userMessageId,
            role: "user",
            content: action.content,
            status: "final",
          },
        ],
        pendingUserMessageId: userMessageId,
      };
    }
    case "job_created":
      return {
        ...state,
        activeJobId: action.jobId,
        activeRunId: action.runId,
        resumedFromRunId: action.resumedFromRunId ?? null,
        busy: true,
        statusText: "Streaming run events",
        lastSignal: action.resumedFromRunId ? "Resumed run" : "Job created",
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

function prepareRunScopedState(
  state: WorkbenchState,
  preserveTools: boolean,
  statusText: string,
): WorkbenchState {
  return {
    ...state,
    activeJobId: null,
    activeRunId: null,
    resumedFromRunId: null,
    statusText,
    eventCount: 0,
    seenEventSeqs: [],
    lastSignal: statusText,
    busy: false,
    error: null,
    messages: state.messages.map((message) =>
      message.status === "streaming"
        ? { ...message, status: "final" as const }
        : message,
    ),
    pendingUserMessageId: null,
    plan: null,
    planDecisions: [],
    planRevisions: [],
    stepRecords: [],
    historicalTools: preserveTools
      ? [
          ...state.tools.filter(
            (tool) => tool.status === "done" || tool.status === "error",
          ),
          ...state.historicalTools,
        ]
      : [],
    tools: [],
    // Keep a short run trace for the inspector; product chat is the transcript.
    trace: [],
    pendingInputs: [],
  };
}

function applyJobState(
  state: WorkbenchState,
  jobState: JobStateResponse,
): WorkbenchState {
  // Interaction responses can arrive after the SSE stream has already advanced.
  if (
    state.activeJobId === jobState.job_id &&
    jobState.event_count < state.eventCount
  ) {
    return state;
  }

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
    resumedFromRunId: jobState.resumed_from_run_id ?? hydrated.resumedFromRunId,
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
    case "run_started": {
      const runStartedAlreadySeen = state.seenUserMessageRunIds.includes(
        event.run_id,
      );
      return {
        ...next,
        activeJobId: event.job_id,
        activeRunId: event.run_id,
        busy: true,
        error: null,
        statusText: "Run started",
        messages: runStartedAlreadySeen
          ? state.messages
          : applyCanonicalUserMessage(
              state.messages,
              event.user_message,
              state.pendingUserMessageId,
            ),
        pendingUserMessageId: null,
        seenUserMessageRunIds: runStartedAlreadySeen
          ? state.seenUserMessageRunIds
          : [...state.seenUserMessageRunIds, event.run_id],
        trace: prependTrace(state.trace, event.type, `${event.user_message}`),
      };
    }
    case "llm_chunk":
      return {
        ...next,
        messages: appendAssistantDelta(state.messages, event.delta),
      };
    case "model_status":
      return {
        ...next,
        statusText: event.message,
        trace: prependTrace(state.trace, event.type, event.message),
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
        planRevisions: event.plan_revision
          ? appendPlanRevision(state.planRevisions, event.plan_revision)
          : state.planRevisions,
        trace: prependTrace(state.trace, event.type, `${event.plan.steps.length} steps`),
      };
    case "plan_decision": {
      const planDecisions = appendPlanDecision(state.planDecisions, event.record);
      return {
        ...next,
        planDecisions,
        trace:
          planDecisions === state.planDecisions
            ? state.trace
            : prependTrace(
                state.trace,
                event.type,
                `${event.record.decision.kind.replaceAll("_", " ")}: ${truncate(
                  event.record.decision.safe_summary,
                  220,
                )}`,
              ),
      };
    }
    case "plan_revised": {
      const planRevisions = appendPlanRevision(state.planRevisions, event.revision);
      return {
        ...next,
        plan: event.plan,
        planRevisions,
        trace:
          planRevisions === state.planRevisions
            ? state.trace
            : prependTrace(
                state.trace,
                event.type,
                `revision ${event.revision.revision}; ${event.plan.steps.length} remaining step(s)`,
              ),
      };
    }
    case "plan_step_started":
      return {
        ...next,
        plan: updatePlan(state.plan, event.index, event.step, false),
        trace: prependTrace(state.trace, event.type, event.step.title),
      };
    case "step_result": {
      const stepRecords = appendStepRecord(state.stepRecords, event.record);
      if (stepRecords === state.stepRecords) {
        return next;
      }
      const succeeded =
        event.record.status === "succeeded" || event.record.status === "skipped";
      const failed =
        event.record.status === "failed" ||
        event.record.status === "blocked" ||
        event.record.status === "interrupted";
      const plan = state.plan
        ? {
            ...state.plan,
            steps: state.plan.steps.map((step) =>
              step.id === event.record.step_id
                ? { ...step, done: succeeded ? true : step.done }
                : step,
            ),
            current_step: succeeded
              ? (() => {
                  const updated = state.plan.steps.map((step) =>
                    step.id === event.record.step_id ? { ...step, done: true } : step,
                  );
                  const nextIndex = updated.findIndex((step) => !step.done);
                  return nextIndex === -1 ? updated.length : nextIndex;
                })()
              : state.plan.current_step,
          }
        : state.plan;
      const label = succeeded
        ? event.record.summary || event.record.step_id
        : failed
          ? `${event.record.step_id}: ${
              event.record.safe_error_summary || event.record.summary || event.record.status
            }`
          : event.record.summary || event.record.step_id;
      return {
        ...next,
        plan,
        stepRecords,
        trace:
          succeeded || failed
            ? prependTrace(state.trace, event.type, label)
            : state.trace,
      };
    }
    case "prompt_compacted":
      return {
        ...next,
        statusText: event.state.degraded
          ? "Prompt compacted with fallback summary"
          : "Prompt compacted",
        trace: prependTrace(
          state.trace,
          event.type,
          formatPromptCompaction(event.summary, event.state),
        ),
      };
    case "prompt_built":
      return {
        ...next,
        trace: prependTrace(
          state.trace,
          event.type,
          formatPromptBuildMetadata(event.metadata),
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
    default:
      return next;
  }
}

function appendStepRecord(records: StepRecord[], record: StepRecord): StepRecord[] {
  return records.some((saved) => saved.record_id === record.record_id)
    ? records
    : [...records, record];
}

function appendPlanDecision(
  records: PlanDecisionRecord[],
  record: PlanDecisionRecord,
): PlanDecisionRecord[] {
  return records.some(
    (saved) =>
      saved.decision.decision_id === record.decision.decision_id ||
      saved.trigger_step_record_id === record.trigger_step_record_id,
  )
    ? records
    : [...records, record];
}

function appendPlanRevision(revisions: PlanRevision[], revision: PlanRevision): PlanRevision[] {
  return revisions.some((saved) => saved.revision_id === revision.revision_id)
    ? revisions
    : [...revisions, revision];
}

function applyCanonicalUserMessage(
  messages: ChatMessage[],
  content: string,
  pendingUserMessageId: string | null,
): ChatMessage[] {
  if (
    pendingUserMessageId &&
    messages.some((message) => message.id === pendingUserMessageId)
  ) {
    return messages.map((message) =>
      message.id === pendingUserMessageId
        ? { ...message, content, status: "final" as const }
        : message,
    );
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

function formatPromptCompaction(
  summary: string | null | undefined,
  state: Extract<StreamEvent, { type: "prompt_compacted" }>["state"],
): string {
  const source = `${state.source_message_count} message(s)`;
  const mode = state.mode.replaceAll("_", " ");
  const fallback = state.degraded ? " degraded" : "";
  const summaryText = summary ? `: ${truncate(summary, 160)}` : "";
  return `${mode}${fallback}; ${source}${summaryText}`;
}

function formatPromptBuildMetadata(
  metadata: Extract<StreamEvent, { type: "prompt_built" }>["metadata"],
): string {
  const history = `${metadata.included_history_messages}/${metadata.dropped_history_messages}`;
  return `${metadata.token_estimate} tokens; history ${history}; ${metadata.prompt_hash}`;
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
