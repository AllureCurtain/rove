import type {
  ApprovalDecision,
  PendingInput,
  PlanDecisionRecord,
  PlanRevision,
  PlanStep,
  PromptBuildMetadata,
  PromptCompactionState,
  PendingApproval,
  StepRecord,
  StreamEvent,
  TaskPlan,
  ToolArtifactRef,
  ToolExecutionMetadata,
  ToolError,
  ToolMutation,
  ToolResultOutcome,
  Usage,
  JobStateResponse,
} from "./rove-types";

export interface ChatMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
  status: "streaming" | "final";
  usage?: Usage;
  promptBuild?: PromptBuildMetadata;
  promptCompaction?: PromptCompactionState;
}

export type ToolExecutionViewMetadata = Omit<
  ToolExecutionMetadata,
  "affected_paths" | "diff_summary"
> & {
  affected_paths: string[];
  diff_summary: string[];
};

export interface ToolCallView {
  id: string;
  /** Stable run-qualified identity used by the ordered transcript projection. */
  timelineId?: string;
  name: string;
  status: "running" | "waiting" | "done" | "error";
  details: string;
  args?: unknown;
  output?: string;
  error?: ToolError;
  mutations?: ToolMutation[];
  metadata?: ToolExecutionViewMetadata;
  reason?: string;
  pendingApproval?: PendingApproval;
  /** Durable artifacts this call produced, in the order they were stored. */
  artifacts?: ToolArtifactRef[];
  /** Artifacts a quota refused, kept so a UI can explain a missing payload. */
  rejectedArtifacts?: RejectedArtifactView[];
  /** Rich outcome when the tool produced an envelope. */
  outcome?: ToolResultOutcome;
}

export interface RejectedArtifactView {
  blockOrdinal: number;
  reason: string;
  observedBytes: number;
}

export interface TraceEntry {
  id: string;
  label: string;
  detail: string;
}

export type TranscriptTimelineEntryKind = "message" | "tool" | "input";

/**
 * Stable presentation index over canonical reducer state. Event payloads remain
 * owned by messages/tools/inputs; this index only records their canonical order.
 */
export interface TranscriptTimelineEntry {
  id: string;
  kind: TranscriptTimelineEntryKind;
  entityId: string;
  runId: string | null;
  runOrdinal: number | null;
  eventSeq: number | null;
  /** A forked session projects these facts from an immutable parent run. */
  inherited: boolean;
  sourceSessionId: string | null;
}

export interface TranscriptInputView {
  id: string;
  timelineId: string;
  prompt: string;
  status: "waiting" | "submitted" | "closed";
}

export type TranscriptTimelineItem =
  | {
      entry: TranscriptTimelineEntry;
      kind: "message";
      message: ChatMessage;
    }
  | {
      entry: TranscriptTimelineEntry;
      kind: "tool";
      tool: ToolCallView;
    }
  | {
      entry: TranscriptTimelineEntry;
      kind: "input";
      input: TranscriptInputView;
    };

export interface TranscriptRunGroup {
  id: string;
  runId: string | null;
  runOrdinal: number | null;
  inherited: boolean;
  sourceSessionId: string | null;
  items: TranscriptTimelineItem[];
}

export interface WorkbenchState {
  activeJobId: string | null;
  activeRunId: string | null;
  activeRunOrdinal: number | null;
  resumedFromRunId: string | null;
  statusText: string;
  eventCount: number;
  seenEventSeqs: number[];
  lastSignal: string;
  busy: boolean;
  error: string | null;
  messages: ChatMessage[];
  timeline: TranscriptTimelineEntry[];
  /** Optimistic user bubble awaiting its canonical run_started identity. */
  pendingUserMessageId: string | null;
  /** Run identities whose canonical user bubble has already been projected. */
  seenUserMessageRunIds: string[];
  runUsage: Usage;
  promptBuild: PromptBuildMetadata | null;
  promptCompaction: PromptCompactionState | null;
  plan: TaskPlan | null;
  planDecisions: PlanDecisionRecord[];
  planRevisions: PlanRevision[];
  stepRecords: StepRecord[];
  /** Terminal tool cards retained from earlier product-session runs. */
  historicalTools: ToolCallView[];
  tools: ToolCallView[];
  trace: TraceEntry[];
  pendingInputs: PendingInput[];
  transcriptInputs: TranscriptInputView[];
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
      runOrdinal?: number | null;
    }
  | {
      type: "append_report_fallback";
      runId: string;
      runOrdinal?: number | null;
      content: string;
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
    activeRunOrdinal: null,
    resumedFromRunId: null,
    statusText: "No active run",
    eventCount: 0,
    seenEventSeqs: [],
    lastSignal: "Idle",
    busy: false,
    error: null,
    messages: [],
    timeline: [],
    pendingUserMessageId: null,
    seenUserMessageRunIds: [],
    runUsage: emptyUsage(),
    promptBuild: null,
    promptCompaction: null,
    plan: null,
    planDecisions: [],
    planRevisions: [],
    stepRecords: [],
    historicalTools: [],
    tools: [],
    trace: [],
    pendingInputs: [],
    transcriptInputs: [],
  };
}

export function selectTranscriptTimeline(
  state: WorkbenchState,
): TranscriptRunGroup[] {
  const messages = new Map(state.messages.map((message) => [message.id, message]));
  const tools = new Map<string, ToolCallView>();
  for (const tool of [...state.historicalTools, ...state.tools]) {
    tools.set(tool.timelineId ?? tool.id, tool);
  }
  const inputs = new Map(
    state.transcriptInputs.map((input) => [input.timelineId, input]),
  );
  const groups: TranscriptRunGroup[] = [];

  const orderedTimeline = state.timeline
    .map((entry, originalIndex) => ({ entry, originalIndex }))
    .sort(compareTimelineEntries)
    .map(({ entry }) => entry);

  for (const entry of orderedTimeline) {
    let item: TranscriptTimelineItem | null = null;
    switch (entry.kind) {
      case "message": {
        const message = messages.get(entry.entityId);
        if (message) {
          item = { entry, kind: "message", message };
        }
        break;
      }
      case "tool": {
        const tool = tools.get(entry.entityId);
        if (tool) {
          item = { entry, kind: "tool", tool };
        }
        break;
      }
      case "input": {
        const input = inputs.get(entry.entityId);
        if (input) {
          item = { entry, kind: "input", input };
        }
        break;
      }
    }
    if (!item) {
      continue;
    }

    const groupId = entry.runId
      ? `run:${entry.runId}`
      : entry.runOrdinal
        ? `ordinal:${entry.runOrdinal}`
        : "run:unbound";
    const lastGroup = groups.at(-1);
    if (lastGroup?.id === groupId) {
      lastGroup.items.push(item);
    } else {
      groups.push({
        id: groupId,
        runId: entry.runId,
        runOrdinal: entry.runOrdinal,
        inherited: entry.inherited,
        sourceSessionId: entry.sourceSessionId,
        items: [item],
      });
    }
  }

  return groups;
}

function compareTimelineEntries(
  left: { entry: TranscriptTimelineEntry; originalIndex: number },
  right: { entry: TranscriptTimelineEntry; originalIndex: number },
): number {
  const leftEntry = left.entry;
  const rightEntry = right.entry;
  if (
    leftEntry.runOrdinal !== null &&
    rightEntry.runOrdinal !== null &&
    leftEntry.runOrdinal !== rightEntry.runOrdinal
  ) {
    return leftEntry.runOrdinal - rightEntry.runOrdinal;
  }
  const sameRun =
    (leftEntry.runId !== null && leftEntry.runId === rightEntry.runId) ||
    (leftEntry.runOrdinal !== null &&
      leftEntry.runOrdinal === rightEntry.runOrdinal);
  if (sameRun && leftEntry.eventSeq !== rightEntry.eventSeq) {
    if (leftEntry.eventSeq === null) {
      return 1;
    }
    if (rightEntry.eventSeq === null) {
      return -1;
    }
    return leftEntry.eventSeq - rightEntry.eventSeq;
  }
  return left.originalIndex - right.originalIndex;
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
      const timelineEntry = transcriptTimelineEntry(
        state,
        "message",
        userMessageId,
        undefined,
        `optimistic:${userMessageId}`,
      );
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
        timeline: appendTimelineEntry(state.timeline, timelineEntry),
        pendingUserMessageId: userMessageId,
      };
    }
    case "job_created":
      return {
        ...state,
        activeJobId: action.jobId,
        activeRunId: action.runId,
        activeRunOrdinal: action.runOrdinal ?? null,
        resumedFromRunId: action.resumedFromRunId ?? null,
        busy: true,
        statusText: "Streaming run events",
        lastSignal: action.resumedFromRunId ? "Resumed run" : "Job created",
        error: null,
      };
    case "append_report_fallback": {
      const messageId = `fallback-${action.runId}`;
      if (state.messages.some((message) => message.id === messageId)) {
        return state;
      }
      const entry = transcriptTimelineEntry(
        {
          ...state,
          activeRunId: action.runId,
          activeRunOrdinal: action.runOrdinal ?? null,
        },
        "message",
        messageId,
        undefined,
        `run:${action.runId}:fallback`,
      );
      return {
        ...state,
        messages: [
          ...state.messages,
          {
            id: messageId,
            role: "assistant",
            content: `Report summary: ${action.content}`,
            status: "final",
          },
        ],
        timeline: appendTimelineEntry(state.timeline, entry),
      };
    }
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
        transcriptInputs: state.transcriptInputs.map((input) =>
          input.id === action.inputId && input.status === "waiting"
            ? { ...input, status: "submitted" as const }
            : input,
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
    activeRunOrdinal: null,
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
    runUsage: emptyUsage(),
    promptBuild: null,
    promptCompaction: null,
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
    transcriptInputs: state.transcriptInputs.map((input) =>
      input.status === "waiting"
        ? { ...input, status: "closed" as const }
        : input,
    ),
  };
}

function runScopeId(state: WorkbenchState): string {
  if (state.activeRunId) {
    return `run:${state.activeRunId}`;
  }
  if (state.activeJobId) {
    return `job:${state.activeJobId}`;
  }
  return "unbound";
}

function transcriptTimelineEntry(
  state: WorkbenchState,
  kind: TranscriptTimelineEntryKind,
  entityId: string,
  eventSeq?: number,
  explicitId?: string,
): TranscriptTimelineEntry {
  const sequenceIdentity = eventSeq === undefined ? "unsequenced" : String(eventSeq);
  return {
    id:
      explicitId ??
      `${runScopeId(state)}:${kind}:${sequenceIdentity}:${entityId}`,
    kind,
    entityId,
    runId: state.activeRunId,
    runOrdinal: state.activeRunOrdinal,
    eventSeq: eventSeq ?? null,
    inherited: false,
    sourceSessionId: null,
  };
}

function canonicalTimelineEntry(
  kind: TranscriptTimelineEntryKind,
  entityId: string,
  runId: string | null,
  runOrdinal: number | null,
  eventSeq: number | undefined,
  factIdentity: string,
): TranscriptTimelineEntry {
  const scope = runId ? `run:${runId}` : "run:unbound";
  const sequenceIdentity = eventSeq === undefined ? "unsequenced" : String(eventSeq);
  return {
    id: `${scope}:${kind}:${sequenceIdentity}:${factIdentity}`,
    kind,
    entityId,
    runId,
    runOrdinal,
    eventSeq: eventSeq ?? null,
    inherited: false,
    sourceSessionId: null,
  };
}

function appendTimelineEntry(
  timeline: TranscriptTimelineEntry[],
  entry: TranscriptTimelineEntry,
): TranscriptTimelineEntry[] {
  const existing = timeline.find(
    (candidate) =>
      candidate.id === entry.id ||
      (candidate.kind === entry.kind && candidate.entityId === entry.entityId),
  );
  return existing ? timeline : [...timeline, entry];
}

function bindTimelineEntry(
  timeline: TranscriptTimelineEntry[],
  entry: TranscriptTimelineEntry,
): TranscriptTimelineEntry[] {
  const index = timeline.findIndex(
    (candidate) =>
      candidate.id === entry.id ||
      (candidate.kind === entry.kind && candidate.entityId === entry.entityId),
  );
  if (index === -1) {
    return [...timeline, entry];
  }
  const existing = timeline[index]!;
  if (
    entry.eventSeq === null ||
    (existing.eventSeq !== null && existing.eventSeq <= entry.eventSeq)
  ) {
    return timeline;
  }
  return [
    ...timeline.slice(0, index),
    entry,
    ...timeline.slice(index + 1),
  ];
}

function toolTimelineEntityId(state: WorkbenchState, callId: string): string {
  return `${runScopeId(state)}:tool:${callId}`;
}

function inputTimelineEntityId(state: WorkbenchState, inputId: string): string {
  return `${runScopeId(state)}:input:${inputId}`;
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
  const jobScopedState = {
    ...hydrated,
    activeJobId: jobState.job_id,
    activeRunId: jobState.run_id,
  };
  const tools = syncPendingApprovals(
    hydrated.tools,
    jobState.pending_approvals,
    jobScopedState,
    busy ? undefined : terminalDetail,
  );
  const pendingInputs = jobState.pending_inputs;
  const transcriptInputs = syncTranscriptInputs(
    hydrated.transcriptInputs,
    jobState.pending_inputs,
    jobScopedState,
  );
  let timeline = hydrated.timeline;
  for (const tool of tools) {
    if (!tool.timelineId) {
      continue;
    }
    timeline = appendTimelineEntry(
      timeline,
      transcriptTimelineEntry(jobScopedState, "tool", tool.timelineId),
    );
  }
  for (const input of transcriptInputs) {
    timeline = appendTimelineEntry(
      timeline,
      transcriptTimelineEntry(jobScopedState, "input", input.timelineId),
    );
  }

  return {
    ...jobScopedState,
    activeJobId: jobState.job_id,
    activeRunId: jobState.run_id,
    resumedFromRunId: jobState.resumed_from_run_id ?? hydrated.resumedFromRunId,
    eventCount: jobState.event_count,
    busy,
    error: jobState.status === "error" ? (state.error ?? "Run failed") : state.error,
    statusText: statusText(jobState.status),
    lastSignal: "Job state synced",
    pendingInputs,
    transcriptInputs,
    tools,
    timeline,
  };
}

function syncPendingApprovals(
  tools: ToolCallView[],
  pendingApprovals: PendingApproval[],
  state: WorkbenchState,
  terminalDetail?: string,
): ToolCallView[] {
  const pendingById = new Map(
    pendingApprovals.map((approval) => [approval.call_id, approval]),
  );
  const existingIds = new Set(tools.map((tool) => tool.id));
  const synced = tools.map((tool) => {
    const pending = pendingById.get(tool.id);
    if (pending) {
      return toolFromPendingApproval(pending, state, tool);
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
    .map((approval) => toolFromPendingApproval(approval, state));
  return [...inserted, ...synced];
}

function toolFromPendingApproval(
  pendingApproval: PendingApproval,
  state: WorkbenchState,
  existing?: ToolCallView,
): ToolCallView {
  return {
    ...existing,
    id: pendingApproval.call_id,
    timelineId:
      existing?.timelineId ?? toolTimelineEntityId(state, pendingApproval.call_id),
    name: pendingApproval.name,
    status: "waiting",
    details: pendingApproval.reason,
    args: pendingApproval.args,
    reason: pendingApproval.reason,
    pendingApproval,
  };
}

function syncTranscriptInputs(
  existing: TranscriptInputView[],
  pending: PendingInput[],
  state: WorkbenchState,
): TranscriptInputView[] {
  const pendingById = new Map(pending.map((input) => [input.input_id, input]));
  const synced = existing.map((input) => {
    const current = pendingById.get(input.id);
    if (current) {
      return { ...input, prompt: current.prompt, status: "waiting" as const };
    }
    return input.status === "waiting"
      ? { ...input, status: "closed" as const }
      : input;
  });
  const existingIds = new Set(existing.map((input) => input.id));
  return [
    ...synced,
    ...pending
      .filter((input) => !existingIds.has(input.input_id))
      .map((input) => ({
        id: input.input_id,
        timelineId: inputTimelineEntityId(state, input.input_id),
        prompt: input.prompt,
        status: "waiting" as const,
      })),
  ];
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
      const userProjection = runStartedAlreadySeen
        ? null
        : applyCanonicalUserMessage(
            state.messages,
            event.user_message,
            state.pendingUserMessageId,
            event.run_id,
          );
      const userTimelineEntry = userProjection
        ? canonicalTimelineEntry(
            "message",
            userProjection.messageId,
            event.run_id,
            state.activeRunOrdinal,
            seq,
            "user",
          )
        : null;
      return {
        ...next,
        activeJobId: event.job_id,
        activeRunId: event.run_id,
        busy: true,
        error: null,
        statusText: "Run started",
        messages: runStartedAlreadySeen
          ? state.messages
          : userProjection!.messages,
        timeline: userTimelineEntry
          ? bindTimelineEntry(state.timeline, userTimelineEntry)
          : state.timeline,
        pendingUserMessageId: null,
        seenUserMessageRunIds: runStartedAlreadySeen
          ? state.seenUserMessageRunIds
          : [...state.seenUserMessageRunIds, event.run_id],
        trace: prependTrace(state.trace, event.type, `${event.user_message}`),
      };
    }
    case "agent_profile_activated":
      return {
        ...next,
        statusText: `Agent ${event.identity.display_name} activated`,
        trace: prependTrace(
          state.trace,
          event.type,
          `${event.identity.selector.source}:${event.identity.selector.agent_id} ${event.identity.profile_hash}`,
        ),
      };
    case "workspace_instructions_resolved":
      return {
        ...next,
        trace: prependTrace(
          state.trace,
          event.type,
          `${event.layer_count} layer(s), ${event.rejected_count} rejected`,
        ),
      };
    case "instruction_overlay_applied":
      return {
        ...next,
        trace: prependTrace(
          state.trace,
          event.type,
          `${event.scope} -> ${event.target_path}`,
        ),
      };
    case "procedures_selected":
      return {
        ...next,
        trace: prependTrace(
          state.trace,
          event.type,
          `${event.selected?.length ?? 0} selected, ${event.excluded_count} excluded`,
        ),
      };
    case "procedure_hydrated":
      return {
        ...next,
        trace: prependTrace(
          state.trace,
          event.type,
          `${event.reference.id}@${event.reference.version}${event.truncated ? " (truncated)" : ""}`,
        ),
      };
    case "llm_chunk": {
      const projection = appendAssistantDelta(
        state.messages,
        event.delta,
        state.activeRunId,
        seq ?? next.eventCount,
      );
      return {
        ...next,
        messages: projection.messages,
        timeline: projection.created
          ? appendTimelineEntry(
              state.timeline,
              canonicalTimelineEntry(
                "message",
                projection.messageId,
                state.activeRunId,
                state.activeRunOrdinal,
                seq,
                "assistant",
              ),
            )
          : state.timeline,
      };
    }
    case "model_status":
      return {
        ...next,
        statusText: event.message,
        trace: prependTrace(state.trace, event.type, event.message),
      };
    case "llm_message": {
      const projection = finalizeAssistantMessage(
        state.messages,
        event.full,
        state.activeRunId,
        seq ?? next.eventCount,
        {
          usage: event.usage,
          promptBuild: state.promptBuild,
          promptCompaction: state.promptCompaction,
        },
      );
      return {
        ...next,
        runUsage: addUsage(state.runUsage, event.usage),
        messages: projection.messages,
        timeline: projection.created
          ? appendTimelineEntry(
              state.timeline,
              canonicalTimelineEntry(
                "message",
                projection.messageId,
                state.activeRunId,
                state.activeRunOrdinal,
                seq,
                "assistant",
              ),
            )
          : state.timeline,
      };
    }
    case "tool_call_started": {
      const timelineId = toolTimelineEntityId(state, event.call_id);
      return {
        ...next,
        tools: upsertTool(state.tools, {
          id: event.call_id,
          timelineId,
          name: event.name,
          status: "running",
          details: formatValue(event.args),
          args: event.args,
        }),
        timeline: bindTimelineEntry(
          state.timeline,
          canonicalTimelineEntry(
            "tool",
            timelineId,
            state.activeRunId,
            state.activeRunOrdinal,
            seq,
            event.call_id,
          ),
        ),
        trace: prependTrace(
          state.trace,
          event.type,
          `${event.name} ${formatValue(event.args)}`,
        ),
      };
    }
    case "tool_call_approval_needed": {
      const timelineId = toolTimelineEntityId(state, event.call_id);
      return {
        ...next,
        tools: upsertTool(state.tools, {
          id: event.call_id,
          timelineId,
          name: event.name,
          status: "waiting",
          details: event.reason,
          args: event.args,
          reason: event.reason,
          pendingApproval: {
            call_id: event.call_id,
            name: event.name,
            args: event.args,
            reason: event.reason,
          },
        }),
        timeline: bindTimelineEntry(
          state.timeline,
          canonicalTimelineEntry(
            "tool",
            timelineId,
            state.activeRunId,
            state.activeRunOrdinal,
            seq,
            event.call_id,
          ),
        ),
        trace: prependTrace(
          state.trace,
          event.type,
          `${event.name} ${event.reason}`,
        ),
      };
    }
    case "tool_call_completed": {
      const timelineId = toolTimelineEntityId(state, event.call_id);
      return {
        ...next,
        tools: upsertTool(state.tools, {
          id: event.call_id,
          timelineId,
          name: findToolName(state.tools, event.result.call_id),
          status: "done",
          details: event.result.output,
          output: event.result.output,
          mutations: event.result.mutations,
          metadata: normalizeToolExecutionMetadata(event.result.metadata),
          // The envelope's own artifacts are authoritative for the completed
          // call. Per-artifact events may arrive first; both converge here.
          artifacts: event.result.envelope?.artifacts,
          outcome: event.result.envelope?.outcome,
        }),
        timeline: bindTimelineEntry(
          state.timeline,
          canonicalTimelineEntry(
            "tool",
            timelineId,
            state.activeRunId,
            state.activeRunOrdinal,
            seq,
            event.call_id,
          ),
        ),
        trace: prependTrace(
          state.trace,
          event.type,
          truncate(event.result.output, 220),
        ),
      };
    }
    // Artifact events accumulate onto the owning call. They never change its
    // status: an artifact is evidence about a call, not its outcome.
    case "tool_artifact_stored": {
      const existing = state.tools.find((tool) => tool.id === event.call_id);
      if (!existing) {
        return next;
      }
      const alreadyRecorded = (existing.artifacts ?? []).some(
        (artifact) => artifact.artifact_id === event.artifact.artifact_id,
      );
      return {
        ...next,
        tools: alreadyRecorded
          ? state.tools
          : upsertTool(state.tools, {
              ...existing,
              artifacts: [...(existing.artifacts ?? []), event.artifact],
            }),
      };
    }
    case "tool_artifact_rejected": {
      const existing = state.tools.find((tool) => tool.id === event.call_id);
      if (!existing) {
        return next;
      }
      const rejection: RejectedArtifactView = {
        blockOrdinal: event.block_ordinal,
        reason: event.reason,
        observedBytes: event.observed_bytes,
      };
      const alreadyRecorded = (existing.rejectedArtifacts ?? []).some(
        (entry) =>
          entry.blockOrdinal === rejection.blockOrdinal &&
          entry.reason === rejection.reason,
      );
      return {
        ...next,
        tools: alreadyRecorded
          ? state.tools
          : upsertTool(state.tools, {
              ...existing,
              rejectedArtifacts: [
                ...(existing.rejectedArtifacts ?? []),
                rejection,
              ],
            }),
        trace: prependTrace(
          state.trace,
          event.type,
          `block ${event.block_ordinal}: ${event.reason}`,
        ),
      };
    }
    case "mcp_server_degraded":
      return {
        ...next,
        trace: prependTrace(
          state.trace,
          event.type,
          `${event.server_config_id}: ${event.failure_code}`,
        ),
      };
    case "mcp_capabilities_refreshed":
      return {
        ...next,
        trace: prependTrace(
          state.trace,
          event.type,
          `${event.server_config_id}: +${event.added.length} -${event.removed.length} ~${event.changed.length}`,
        ),
      };
    case "tool_call_failed": {
      const timelineId = toolTimelineEntityId(state, event.call_id);
      return {
        ...next,
        tools: upsertTool(state.tools, {
          id: event.call_id,
          timelineId,
          name: findToolName(state.tools, event.call_id),
          status: "error",
          details: formatToolError(event.error),
          error: event.error,
          metadata: normalizeToolExecutionMetadata(event.metadata),
        }),
        timeline: bindTimelineEntry(
          state.timeline,
          canonicalTimelineEntry(
            "tool",
            timelineId,
            state.activeRunId,
            state.activeRunOrdinal,
            seq,
            event.call_id,
          ),
        ),
        trace: prependTrace(
          state.trace,
          event.type,
          formatToolError(event.error),
        ),
      };
    }
    case "input_needed": {
      const timelineId = inputTimelineEntityId(state, event.input_id);
      return {
        ...next,
        pendingInputs: [
          ...state.pendingInputs.filter((input) => input.input_id !== event.input_id),
          { input_id: event.input_id, prompt: event.prompt },
        ],
        transcriptInputs: upsertTranscriptInput(state.transcriptInputs, {
          id: event.input_id,
          timelineId,
          prompt: event.prompt,
          status: "waiting",
        }),
        timeline: bindTimelineEntry(
          state.timeline,
          canonicalTimelineEntry(
            "input",
            timelineId,
            state.activeRunId,
            state.activeRunOrdinal,
            seq,
            event.input_id,
          ),
        ),
        trace: prependTrace(state.trace, event.type, event.prompt),
      };
    }
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
        promptCompaction: event.state,
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
        promptBuild: event.metadata,
        trace: prependTrace(
          state.trace,
          event.type,
          formatPromptBuildMetadata(event.metadata),
        ),
      };
    case "run_completed": {
      const projection = finalizeRunMessage(
        state.messages,
        event.output,
        state.activeRunId,
        seq ?? next.eventCount,
      );
      return {
        ...next,
        busy: false,
        statusText: `Run completed: ${event.reason}`,
        lastSignal: `Run completed`,
        messages: projection.messages,
        timeline: projection.created
          ? appendTimelineEntry(
              state.timeline,
              canonicalTimelineEntry(
                "message",
                projection.messageId,
                state.activeRunId,
                state.activeRunOrdinal,
                seq,
                "assistant",
              ),
            )
          : state.timeline,
        pendingInputs: [],
        transcriptInputs: state.transcriptInputs.map((input) =>
          input.status === "waiting"
            ? { ...input, status: "closed" as const }
            : input,
        ),
        trace: prependTrace(
          state.trace,
          event.type,
          event.output ? truncate(event.output, 220) : event.reason,
        ),
      };
    }
    case "steer_accepted":
      return {
        ...next,
        statusText: "Steer accepted at a safe point",
        trace: prependTrace(state.trace, event.type, truncate(event.content, 220)),
      };
    case "steer_applied":
      return {
        ...next,
        statusText: "Steer applied to the next model turn",
        trace: prependTrace(state.trace, event.type, event.id),
      };
    case "steer_dropped":
      return {
        ...next,
        trace: prependTrace(state.trace, event.type, event.reason),
      };
    case "followup_queued":
      return {
        ...next,
        trace: prependTrace(state.trace, event.type, truncate(event.content, 220)),
      };
    case "followup_dequeued":
      return {
        ...next,
        statusText: "Starting queued follow-up",
        trace: prependTrace(state.trace, event.type, event.id),
      };
    case "followup_abandoned":
      return {
        ...next,
        trace: prependTrace(state.trace, event.type, event.reason),
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

interface MessageProjection {
  messages: ChatMessage[];
  messageId: string;
  created: boolean;
}

function applyCanonicalUserMessage(
  messages: ChatMessage[],
  content: string,
  pendingUserMessageId: string | null,
  runId: string,
): MessageProjection {
  if (
    pendingUserMessageId &&
    messages.some((message) => message.id === pendingUserMessageId)
  ) {
    return {
      messages: messages.map((message) =>
        message.id === pendingUserMessageId
          ? { ...message, content, status: "final" as const }
          : message,
      ),
      messageId: pendingUserMessageId,
      created: false,
    };
  }
  const messageId = `message:${runId}:user`;
  return {
    messages: [
      ...messages,
      {
        id: messageId,
        role: "user",
        content,
        status: "final",
      },
    ],
    messageId,
    created: true,
  };
}

function appendAssistantDelta(
  messages: ChatMessage[],
  delta: string,
  runId: string | null,
  eventIdentity: number,
): MessageProjection {
  if (delta.length === 0) {
    return {
      messages,
      messageId: messages.at(-1)?.id ?? "",
      created: false,
    };
  }
  const last = messages[messages.length - 1];
  if (last?.role === "assistant" && last.status === "streaming") {
    return {
      messages: [
        ...messages.slice(0, -1),
        {
          ...last,
          content: last.content + delta,
        },
      ],
      messageId: last.id,
      created: false,
    };
  }
  const messageId = assistantMessageId(runId, eventIdentity);
  return {
    messages: [
      ...messages,
      {
        id: messageId,
        role: "assistant",
        content: delta,
        status: "streaming",
      },
    ],
    messageId,
    created: true,
  };
}

function finalizeAssistantMessage(
  messages: ChatMessage[],
  full: string,
  runId: string | null,
  eventIdentity: number,
  evidence: {
    usage: Usage;
    promptBuild: PromptBuildMetadata | null;
    promptCompaction: PromptCompactionState | null;
  },
): MessageProjection {
  const last = messages[messages.length - 1];
  if (last?.role === "assistant" && last.status === "streaming") {
    return {
      messages: [
        ...messages.slice(0, -1),
        {
          ...last,
          content: full,
          status: "final",
          usage: evidence.usage,
          promptBuild: evidence.promptBuild ?? undefined,
          promptCompaction: evidence.promptCompaction ?? undefined,
        },
      ],
      messageId: last.id,
      created: false,
    };
  }
  if (
    last?.role === "assistant" &&
    last.status === "final" &&
    last.content === full
  ) {
    // Duplicate completion for a message already finalized by a later segment
    // (possible on overlapping restore ranges). Omitted facts stay absent so
    // the replay cannot stamp a misleading zero-usage read onto it.
    return {
      messages,
      messageId: last.id,
      created: false,
    };
  }
  const messageId = assistantMessageId(runId, eventIdentity);
  return {
    messages: [
      ...messages,
      {
        id: messageId,
        role: "assistant",
        content: full,
        status: "final",
        usage: evidence.usage,
        promptBuild: evidence.promptBuild ?? undefined,
        promptCompaction: evidence.promptCompaction ?? undefined,
      },
    ],
    messageId,
    created: true,
  };
}

function finalizeRunMessage(
  messages: ChatMessage[],
  output: string | null | undefined,
  runId: string | null,
  eventIdentity: number,
): MessageProjection {
  if (!output) {
    return {
      messages,
      messageId: messages.at(-1)?.id ?? "",
      created: false,
    };
  }
  const last = messages[messages.length - 1];
  if (last?.role === "assistant" && last.status === "streaming") {
    return {
      messages: [
        ...messages.slice(0, -1),
        {
          ...last,
          content: output,
          status: "final",
        },
      ],
      messageId: last.id,
      created: false,
    };
  }
  if (last?.role === "assistant" && last.content === output) {
    return {
      messages,
      messageId: last.id,
      created: false,
    };
  }
  const messageId = assistantMessageId(runId, eventIdentity);
  return {
    messages: [
      ...messages,
      {
        id: messageId,
        role: "assistant",
        content: output,
        status: "final",
      },
    ],
    messageId,
    created: true,
  };
}

function assistantMessageId(runId: string | null, eventIdentity: number): string {
  return `message:${runId ?? "unbound"}:assistant:${eventIdentity}`;
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

function normalizeToolExecutionMetadata(
  metadata: ToolExecutionMetadata | undefined,
): ToolExecutionViewMetadata | undefined {
  if (!metadata) {
    return undefined;
  }
  return {
    ...metadata,
    affected_paths: Array.isArray(metadata.affected_paths)
      ? metadata.affected_paths
      : [],
    diff_summary: Array.isArray(metadata.diff_summary)
      ? metadata.diff_summary
      : [],
  };
}

function upsertTranscriptInput(
  inputs: TranscriptInputView[],
  next: TranscriptInputView,
): TranscriptInputView[] {
  const index = inputs.findIndex((input) => input.timelineId === next.timelineId);
  if (index === -1) {
    return [...inputs, next];
  }
  return [
    ...inputs.slice(0, index),
    { ...inputs[index]!, ...next },
    ...inputs.slice(index + 1),
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

function emptyUsage(): Usage {
  return {
    prompt_tokens: 0,
    completion_tokens: 0,
    total_tokens: 0,
    cached_tokens: 0,
  };
}

function addUsage(current: Usage, next: Usage): Usage {
  return {
    prompt_tokens: current.prompt_tokens + next.prompt_tokens,
    completion_tokens: current.completion_tokens + next.completion_tokens,
    total_tokens: current.total_tokens + next.total_tokens,
    cached_tokens: (current.cached_tokens ?? 0) + (next.cached_tokens ?? 0),
  };
}

function truncate(value: string, max: number): string {
  return value.length <= max ? value : `${value.slice(0, max)}…`;
}

function humanizeEventName(value: string): string {
  return value
    .replaceAll("_", " ")
    .replace(/\b\w/g, (match) => match.toUpperCase());
}
