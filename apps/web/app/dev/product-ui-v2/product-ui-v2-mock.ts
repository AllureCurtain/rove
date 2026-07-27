export type MockSessionStatus = "running" | "complete" | "attention";
export type MockTimelineState = "message" | "complete" | "attention" | "running";

export const PRODUCT_UI_V2_MOCK_WORKSPACE_ROOT = "mock-workspace/rove";
export const PRODUCT_UI_V2_PREVIEW_BOUNDARY =
  "Inert design mock. No API, persistence, or real approvals.";

type MockMessageEntry = {
  id: string;
  kind: "message";
  meta: string;
  state: MockTimelineState;
  event?: string;
  actor: "user" | "assistant";
  byline: string;
  detail: string;
  text: string;
  streaming?: boolean;
};

type MockEventEntry = {
  id: string;
  kind: "event";
  meta: string;
  state: Exclude<MockTimelineState, "message">;
  event: string;
  title: string;
  detail: string;
  tag?: string;
};

type MockToolEntry = {
  id: string;
  kind: "tool";
  meta: string;
  state: "complete" | "attention";
  event: string;
  title: string;
  subtitle: string;
  outcome: string;
  command: string;
  facts: ReadonlyArray<{ label: string; value: string }>;
};

type MockApprovalEntry = {
  id: string;
  kind: "approval";
  meta: string;
  state: "attention";
  event: string;
  description: string;
  risk: string;
  facts: ReadonlyArray<{ label: string; value: string }>;
};

export type MockTranscriptEntry =
  | MockMessageEntry
  | MockEventEntry
  | MockToolEntry
  | MockApprovalEntry;

type MockPlanItem = {
  id: string;
  label: string;
  state: "complete" | "active" | "pending";
};

export type ProductUiV2MockSession = {
  id: string;
  title: string;
  branch: string;
  status: MockSessionStatus;
  statusLabel: string;
  updatedAt: string;
  updatedDateTime: string;
  headerStatusLabel: string;
  transcript: ReadonlyArray<MockTranscriptEntry>;
  inspector: {
    heading: string;
    runDetail: string;
    planStatus: string;
    planItems: ReadonlyArray<MockPlanItem>;
    facts: ReadonlyArray<{ label: string; value: string }>;
    events: ReadonlyArray<{
      id: string;
      label: string;
      state: "complete" | "attention" | "active";
    }>;
  };
  composer: {
    statusLabel: string;
    helper: string;
    placeholder: string;
    canSteer: boolean;
    canStop: boolean;
  };
};

export const PRODUCT_UI_V2_MOCK_SESSIONS = [
  {
    id: "session-c4-web-control",
    title: "C4 web control surface",
    branch: "main",
    status: "running",
    statusLabel: "Running",
    updatedAt: "now",
    updatedDateTime: "2026-07-27T09:43:00+08:00",
    headerStatusLabel: "Run active",
    transcript: [
      {
        id: "c4-user-request",
        kind: "message",
        meta: "09:41",
        state: "message",
        actor: "user",
        byline: "You",
        detail: "workspace owner",
        text: "Audit the Web control surface against pi-web. Keep Rove's exact session continuity and approval boundary, then propose the smallest safe C4 slice.",
      },
      {
        id: "c4-assistant-ack",
        kind: "message",
        meta: "09:41",
        state: "message",
        actor: "assistant",
        byline: "Rove",
        detail: "assistant",
        text: "I'll verify the current runtime contract first, then compare interaction capability. I will treat canonical events as evidence and keep speculative UI state visibly separate.",
      },
      {
        id: "c4-workspace-resolved",
        kind: "event",
        meta: "09:42",
        state: "complete",
        event: "workspace.resolved",
        title: "Workspace boundary resolved",
        detail: "Folder root is exact. Session binding is server-owned.",
        tag: "main",
      },
      {
        id: "c4-read-contracts",
        kind: "tool",
        meta: "09:42",
        state: "complete",
        event: "tool.completed",
        title: "Read current Web contracts",
        subtitle: "fs.read, bounded to workspace",
        outcome: "Completed",
        command: 'rg "canonical|resume|approval" apps/web docs/runtime',
        facts: [
          { label: "Scope", value: "Workspace read only" },
          { label: "Evidence", value: "Canonical transcript projection found" },
          { label: "Mutation", value: "None" },
        ],
      },
      {
        id: "c4-approval",
        kind: "approval",
        meta: "09:43",
        state: "attention",
        event: "approval.requested",
        description: "Change the Memory endpoint to resolve the selected workspace.",
        risk: "workspace mutation",
        facts: [
          { label: "Path", value: "apps/api/src/product/platform.rs" },
          { label: "Effect", value: "Source edit plus focused tests" },
          { label: "Boundary", value: "Selected workspace only" },
        ],
      },
      {
        id: "c4-assistant-delta",
        kind: "message",
        meta: "now",
        state: "running",
        event: "assistant.delta",
        actor: "assistant",
        byline: "Rove",
        detail: "working",
        text: "The first implementation slice should repair workspace-scoped Memory and cancellation reconciliation before adding richer rendering. This keeps the safety contract ahead of surface polish.",
        streaming: true,
      },
    ],
    inspector: {
      heading: "Current execution",
      runDetail: "active and observed",
      planStatus: "in progress",
      planItems: [
        { id: "verify-contracts", label: "Verify current contracts", state: "complete" },
        { id: "compare-behavior", label: "Compare interaction behavior", state: "complete" },
        { id: "repair-gaps", label: "Repair correctness gaps", state: "active" },
        { id: "upgrade-rendering", label: "Upgrade conversation rendering", state: "pending" },
      ],
      facts: [
        { label: "Approval", value: "Ask for mutations" },
        { label: "Context", value: "Provider report pending" },
        { label: "Cost", value: "Unavailable" },
        { label: "Side effects", value: "No completed mutation" },
      ],
      events: [
        { id: "workspace-resolved", label: "workspace.resolved", state: "complete" },
        { id: "tool-completed", label: "tool.completed", state: "complete" },
        { id: "approval-requested", label: "approval.requested", state: "attention" },
        { id: "assistant-delta", label: "assistant.delta", state: "active" },
      ],
    },
    composer: {
      statusLabel: "Run is active",
      helper: "Steer now or queue a follow-up",
      placeholder: "Steer the active run...",
      canSteer: true,
      canStop: true,
    },
  },
  {
    id: "session-memory-scope-audit",
    title: "Memory scope audit",
    branch: "main",
    status: "complete",
    statusLabel: "Completed",
    updatedAt: "18 min",
    updatedDateTime: "2026-07-27T09:25:00+08:00",
    headerStatusLabel: "Run completed",
    transcript: [
      {
        id: "memory-user-request",
        kind: "message",
        meta: "09:18",
        state: "message",
        actor: "user",
        byline: "You",
        detail: "workspace owner",
        text: "Verify which memory layers are visible to this workspace and confirm that no repository instructions are treated as permission.",
      },
      {
        id: "memory-assistant-ack",
        kind: "message",
        meta: "09:18",
        state: "message",
        actor: "assistant",
        byline: "Rove",
        detail: "assistant",
        text: "I will compare the selected workspace, layered memory files, and runtime policy as separate authorities. This audit will remain read only.",
      },
      {
        id: "memory-workspace-resolved",
        kind: "event",
        meta: "09:19",
        state: "complete",
        event: "workspace.resolved",
        title: "Selected workspace confirmed",
        detail: "The memory audit is bound to the exact rove root.",
        tag: "exact root",
      },
      {
        id: "memory-compare-scopes",
        kind: "tool",
        meta: "09:21",
        state: "complete",
        event: "tool.completed",
        title: "Compare memory scopes",
        subtitle: "fs.read, three bounded sources",
        outcome: "Completed",
        command: 'rg "authority|workspace|memory" MEMORY_DOCTRINE.md runtime/src/memory',
        facts: [
          { label: "Sources", value: "Workspace, user, runtime policy" },
          { label: "Authority", value: "Kept distinct" },
          { label: "Mutation", value: "None" },
        ],
      },
      {
        id: "memory-scope-checked",
        kind: "event",
        meta: "09:24",
        state: "complete",
        event: "memory.scope.checked",
        title: "Memory boundaries agree",
        detail: "Retrieved context remains evidence and cannot grant tool permission.",
        tag: "3 layers",
      },
      {
        id: "memory-assistant-complete",
        kind: "message",
        meta: "09:25",
        state: "complete",
        event: "assistant.completed",
        actor: "assistant",
        byline: "Rove",
        detail: "completed",
        text: "The workspace, user, and runtime layers resolve independently. No memory text crossed the approval boundary, and the audit produced no side effects.",
      },
    ],
    inspector: {
      heading: "Audit result",
      runDetail: "completed without mutation",
      planStatus: "complete",
      planItems: [
        { id: "resolve-workspace", label: "Resolve selected workspace", state: "complete" },
        { id: "read-layers", label: "Read bounded memory layers", state: "complete" },
        { id: "compare-authority", label: "Compare authority boundaries", state: "complete" },
        { id: "report-audit", label: "Report audit evidence", state: "complete" },
      ],
      facts: [
        { label: "Approval", value: "Not required" },
        { label: "Context", value: "3 memory layers" },
        { label: "Cost", value: "Unavailable" },
        { label: "Side effects", value: "None" },
      ],
      events: [
        { id: "workspace-resolved", label: "workspace.resolved", state: "complete" },
        { id: "tool-completed", label: "tool.completed", state: "complete" },
        { id: "memory-checked", label: "memory.scope.checked", state: "complete" },
        { id: "assistant-completed", label: "assistant.completed", state: "complete" },
      ],
    },
    composer: {
      statusLabel: "Run completed",
      helper: "Start a follow-up from this exact session",
      placeholder: "Queue a follow-up for this session...",
      canSteer: false,
      canStop: false,
    },
  },
  {
    id: "session-provider-retry",
    title: "Provider retry behavior",
    branch: "retry-policy",
    status: "attention",
    statusLabel: "Needs attention",
    updatedAt: "42 min",
    updatedDateTime: "2026-07-27T09:01:00+08:00",
    headerStatusLabel: "Input required",
    transcript: [
      {
        id: "provider-user-request",
        kind: "message",
        meta: "08:56",
        state: "message",
        actor: "user",
        byline: "You",
        detail: "workspace owner",
        text: "Reproduce the configured provider retry path without sending duplicate side effects, then stop if the retry policy becomes ambiguous.",
      },
      {
        id: "provider-assistant-ack",
        kind: "message",
        meta: "08:56",
        state: "message",
        actor: "assistant",
        byline: "Rove",
        detail: "assistant",
        text: "I will use the bounded fake-provider path and preserve the original request identity across retries. I will pause before choosing an ambiguous recovery action.",
      },
      {
        id: "provider-workspace-resolved",
        kind: "event",
        meta: "08:57",
        state: "complete",
        event: "workspace.resolved",
        title: "Retry fixture resolved",
        detail: "The deterministic provider fixture is active for this session.",
        tag: "fake provider",
      },
      {
        id: "provider-probe-route",
        kind: "tool",
        meta: "08:59",
        state: "attention",
        event: "provider.retry_wait",
        title: "Probe provider retry route",
        subtitle: "attempt identity preserved",
        outcome: "Retry paused",
        command: "cargo test -p rove-integration-tests provider_retry_is_bounded",
        facts: [
          { label: "Attempt", value: "2 of 3" },
          { label: "Request", value: "Original identity retained" },
          { label: "Side effect", value: "None in flight" },
        ],
      },
      {
        id: "provider-budget-paused",
        kind: "event",
        meta: "09:00",
        state: "attention",
        event: "input.required",
        title: "Retry budget needs a decision",
        detail: "The next attempt requires an explicit provider recovery choice.",
        tag: "paused",
      },
      {
        id: "provider-assistant-attention",
        kind: "message",
        meta: "09:01",
        state: "attention",
        event: "assistant.message",
        actor: "assistant",
        byline: "Rove",
        detail: "waiting for input",
        text: "The retry remains fail closed. Choose whether to retry the same profile or end this run; no external request is currently in flight.",
      },
    ],
    inspector: {
      heading: "Retry boundary",
      runDetail: "paused for explicit input",
      planStatus: "input required",
      planItems: [
        { id: "load-profile", label: "Load server-owned profile", state: "complete" },
        { id: "preserve-identity", label: "Preserve request identity", state: "complete" },
        { id: "inspect-retry", label: "Inspect bounded retry result", state: "complete" },
        { id: "choose-recovery", label: "Choose recovery action", state: "active" },
      ],
      facts: [
        { label: "Approval", value: "Not applicable" },
        { label: "Context", value: "Retry 2 of 3" },
        { label: "Cost", value: "Unavailable" },
        { label: "Side effects", value: "No request in flight" },
      ],
      events: [
        { id: "workspace-resolved", label: "workspace.resolved", state: "complete" },
        { id: "provider-retry", label: "provider.retry_wait", state: "attention" },
        { id: "input-required", label: "input.required", state: "attention" },
        { id: "assistant-message", label: "assistant.message", state: "active" },
      ],
    },
    composer: {
      statusLabel: "Input required",
      helper: "Queue guidance for the paused retry boundary",
      placeholder: "Queue provider guidance...",
      canSteer: false,
      canStop: false,
    },
  },
] as const satisfies ReadonlyArray<ProductUiV2MockSession>;

export const INITIAL_PRODUCT_UI_V2_MOCK_SESSION_ID = PRODUCT_UI_V2_MOCK_SESSIONS[0].id;
