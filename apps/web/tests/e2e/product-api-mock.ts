import type { Page, Route } from "@playwright/test";

import type {
  CreateProductMemoryTopicRequest,
  CreateProductMcpServerRequest,
  ProductMemoryTopicContentResponse,
  ProductMcpHealthSnapshot,
  ProductMcpServerConfig,
  ProductMcpToolDescriptor,
  UpdateProductMcpServerRequest,
  UpdateProductMemoryTopicRequest,
  ProductTrustDecisionRequest,
  ProductTrustStatus,
} from "../../settings/settings-platform-api-types";
import { PRODUCT_TRUST_CAPABILITIES } from "../../settings/settings-platform-api-types";
import type {
  M1BrowserMigrationRequest,
  M1BrowserMigrationResponse,
  M1MigrationIssue,
  ProductMessage,
  ProductMessageSearchHit,
  ProductSearchHit,
} from "../../product/product-api-types";

const NOW = "2026-07-26T00:00:00.000Z";

export interface MockWorkspace {
  id: string;
  canonical_root: string;
  kind: "folder" | "repo";
  display_name: string;
  pinned: boolean;
  last_opened_at: string;
  created_at: string;
  updated_at: string;
}

export interface MockSession {
  id: string;
  workspace_id: string;
  title: string;
  status: "idle" | "running" | "error" | "needs_attention" | "archived";
  runtime_binding?: {
    ordinal: number;
    runtime_session_id: string;
    latest_job_id: string;
    latest_run_id: string;
  };
  created_at: string;
  updated_at: string;
  parent_session_id?: string;
  fork_point_run_id?: string;
  fork_point_seq?: number;
  /** R3: the outcome of the most recently finished turn, when there is one. */
  last_outcome?: "success" | "failed" | "cancelled";
  last_outcome_at?: string;
}

export type MockSessionStatus = MockSession["status"];

export interface MockTranscript {
  product_session_id: string;
  workspace_id: string;
  status: "complete" | "partial";
  partial_reasons: Array<Record<string, unknown>>;
  segments: Array<Record<string, unknown>>;
  /** Present on a cursor page only, while strictly older runs remain. */
  next_before_ordinal?: number;
  /** Present on every cursor page; absent on the legacy response. */
  has_more?: boolean;
}

export interface MockProviderProfile {
  id: string;
  label: string;
  provider_type: "openai" | "openai-responses" | "anthropic" | "ollama" | "fake";
  api_base: string;
  api_key_env?: string;
  default_model?: string;
  created_at: string;
  updated_at: string;
  catalog_revision: string;
}

type MockProviderProfileSeed = Omit<MockProviderProfile, "catalog_revision"> & {
  catalog_revision?: string;
};

export interface MockSessionModelConfig {
  product_session_id: string;
  profile_id?: string;
  model: string;
  reasoning: "default" | "low" | "medium" | "high";
  max_steps: number;
  revision: number;
  updated_at: string;
}

export interface MockMcpProbeFailure {
  status: number;
  code: string;
  error: string;
}

export interface MockProductApiOptions {
  workspaces?: MockWorkspace[];
  sessions?: MockSession[];
  transcripts?: Record<string, MockTranscript>;
  providerProfiles?: MockProviderProfileSeed[];
  memoryTopics?: Record<string, ProductMemoryTopicContentResponse>;
  memoryMutationFailures?: number;
  mcpServers?: Record<string, ProductMcpServerConfig[]>;
  mcpHealth?: Record<string, ProductMcpHealthSnapshot[]>;
  mcpMutationFailures?: number;
  mcpProbeFailures?: Record<string, MockMcpProbeFailure[]>;
  mcpProbeTools?: ProductMcpToolDescriptor[];
  activeWorkspaceId?: string;
  activeSessionId?: string;
  /**
   * Seed `preferences.provider_selection`. A `profile_id` that no seeded
   * profile owns is exactly the §9.2 no-model state: the selection survives
   * but cannot be satisfied.
   */
  providerSelection?: {
    profile_id?: string | null;
    model?: string;
    approval?: string;
    max_steps?: number;
  };
  mode?: MockJobMode;
  transcriptDelayMs?: Record<string, number>;
  transcriptFailures?: Record<string, number>;
  sessionModelConfigFailures?: number;
  /**
   * Failures for `GET .../model-config`, per session id. The shell reads the
   * active session's config on load, so a card-only failure has to be addressed
   * to the session the test hovers.
   */
  sessionModelConfigReadFailures?: Record<string, number>;
  disconnectJobStartResponses?: number;
  jobBindingVisibilityDelayReads?: number;
  sessionCreateDelayMs?: number;
  workspaceDeleteDelayMs?: number;
  preferenceUpdateFailures?: number;
  preferenceUpdateDelayMs?: number;
  migrationFailures?: number;
  migrationIssues?: M1MigrationIssue[];
  trustStatuses?: Record<string, ProductTrustStatus>;
  /**
   * F11: sessions the search endpoint knows but the plain list never returns.
   * They stand in for a directory larger than the catalog's page ceiling, which
   * is exactly the case where a local substring filter is not the whole answer.
   */
  searchOnlySessions?: MockSession[];
  /** Failures for `GET /product/sessions` when it carries a `q`. */
  sessionSearchFailures?: number;
  /**
   * R7: the corpus both search routes answer from, keyed by the canonical scope
   * (`session:<id>` covers the session-scoped route and the unified route's
   * session scope alike). Nothing is derived from the transcript: a search answer
   * is asserted as the server sent it, including a legacy trace hit with no
   * `created_at` and a `seq` that is not a message.
   */
  searchCorpus?: Record<string, ProductSearchHit[]>;
  /**
   * R7: cursors the unified route answers with 409 `product_events_expired`, as
   * if the run the cursor pointed at had been cleaned. The first page has no
   * cursor, so "the content is gone" is expressed with `searchExpiredScopes`.
   */
  searchExpiredCursors?: string[];
  searchExpiredScopes?: string[];
  /** Failures for `GET /product/search`, consumed in order. */
  productSearchFailures?: number;
  /** Failures for `GET .../sessions/{id}/search`, consumed in order. */
  sessionMessageSearchFailures?: number;
  /**
   * R7: how many hits the trace scope returns in one request at most, standing in
   * for the shared per-request byte budget of the real route. It is what makes a
   * page come back short *with* a cursor.
   */
  traceSearchPageSize?: number;
  /**
   * R9: the compaction response the next manual call gets. The route refuses with
   * 409 `product_session_active` while a run is live, exactly as the real one does,
   * so a test only supplies the idle answer it wants to render.
   */
  compactionResult?: Partial<MockCompactionResult>;
  /**
   * R6: the typed refusal the next fork gets, standing in for a cut the server
   * will not honour (an unusable target, another run's message, or a live
   * parent). The client's own ledger is unchanged, so the request is still sent
   * with the exact cut it derived — which is the point of the case.
   */
  forkRefusal?: { status: number; code: string };
  /**
   * R2b: the partial text a cancelled run salvages. When set, `POST
   * /jobs/{id}/cancel` appends the terminal `llm_message` the runtime would have
   * written — `aborted: true` carrying the text produced so far — so the salvage
   * is a real event on the wire rather than a flag the client sets for itself.
   */
  salvageOnCancel?: string;
  /**
   * R8: upload refusals, consumed in order. The real route answers a typed code
   * with the matching status, and the composer's rows are asserted against
   * exactly those pairs rather than against a generic failure.
   */
  attachmentFailures?: Array<{ status: number; code: string }>;
  /**
   * R8: warnings the next upload answers with. The real route derives them from
   * the bytes and the name; a test states the answer it wants rendered.
   */
  attachmentWarnings?: string[];
  /**
   * R8: `HEAD` probes that refuse, by attachment id, consumed in order. It is
   * how "the reference a restore brought back is gone" is expressed.
   */
  attachmentProbeFailures?: Record<string, Array<{ status: number; code: string }>>;
  /**
   * R8: the exact response body the next upload gets, overriding everything
   * else. It is how a malformed body reaches the client.
   */
  attachmentUploadBody?: unknown;
  /**
   * R8: the read-time degradation reason the messages route stamps on every
   * available image reference — what the real API does when the session's
   * selected model cannot accept image input.
   */
  attachmentDegradation?: string;
  /**
   * F.5: hold `POST .../controls/{id}/confirm` open for this long, so a test can
   * see the request in flight instead of racing it.
   */
  followupConfirmDelayMs?: number;
  /**
   * F.5: refusals the confirm route answers before it lets one through, in
   * order. The route's real refusals are a moved-on session (409
   * `product_session_active`) and a row that is not an abandoned follow-up.
   */
  followupConfirmFailures?: Array<{
    status: number;
    code: string;
    error: string;
  }>;
}

export interface MockProductApiState {
  workspaces: MockWorkspace[];
  sessions: MockSession[];
  sessionModelConfigs: Record<string, MockSessionModelConfig>;
  transcripts: Record<string, MockTranscript>;
  messages: Record<string, ProductMessage[]>;
  providerProfiles: MockProviderProfile[];
  providerCatalogRevision: string;
  providerProfileMutationRequests: Array<{
    method: "POST" | "PUT" | "DELETE";
    profileId?: string;
    expectedRevision?: string;
  }>;
  memoryTopics: Record<string, ProductMemoryTopicContentResponse>;
  memoryWorkspaceRequests: Array<string | null>;
  memoryMutationRequests: number;
  remainingMemoryMutationFailures: number;
  mcpServers: Record<string, ProductMcpServerConfig[]>;
  mcpHealth: Record<string, ProductMcpHealthSnapshot[]>;
  mcpRequestBodies: string[];
  mcpMutationRequests: number;
  remainingMcpMutationFailures: number;
  mcpProbeFailures: Record<string, MockMcpProbeFailure[]>;
  jobs: Array<Record<string, unknown>>;
  jobEffectiveConfigs: Array<{
    product_session_id: string;
    approval: "ask" | "auto" | "never";
    model: string;
    max_steps: number;
  }>;
  jobStarts: Array<{
    job_id: string;
    run_id: string;
    resumed_from_run_id: string | null;
  }>;
  eventConnections: string[];
  preferences: Record<string, unknown>;
  transcriptFailures: Record<string, number>;
  sessionModelConfigUpdateRequests: number;
  remainingSessionModelConfigFailures: number;
  /** Every `GET .../model-config`, in order, so a caller can be identified. */
  sessionModelConfigReads: string[];
  remainingSessionModelConfigReadFailures: Record<string, number>;
  disconnectedJobStartResponses: number;
  delayedJobBindingReads: number;
  sessionCreateRequests: number;
  preferenceUpdateRequests: number;
  remainingPreferenceUpdateFailures: number;
  migrationRequestBodies: string[];
  remainingMigrationFailures: number;
  initialStateReadRequests: number;
  /**
   * Reads of the product session list, including background status polling.
   * Tests also mutate `sessions[i].status` directly: the next poll is what the
   * shell notices.
   */
  sessionListReads: number;
  /**
   * F11: every `GET /product/sessions` that carried a `q`, in order, with the
   * limit the client asked for. Proves the sidebar asked the server at all.
   */
  sessionSearchRequests: Array<{ q: string; limit: string | null }>;
  searchOnlySessions: MockSession[];
  remainingSessionSearchFailures: number;
  /** R7: every unified search request, in order, as the URL carried it. */
  productSearchRequests: Array<{
    q: string | null;
    scope: string | null;
    cursor: string | null;
    limit: string | null;
  }>;
  /** R7: every session-scoped search request, in order. */
  sessionMessageSearchRequests: Array<{
    sessionId: string;
    q: string | null;
    cursor: string | null;
    limit: string | null;
  }>;
  /** R7: the corpus both search routes answer from, by canonical scope. */
  searchCorpus: Record<string, ProductSearchHit[]>;
  searchExpiredCursors: string[];
  searchExpiredScopes: string[];
  remainingProductSearchFailures: number;
  remainingSessionMessageSearchFailures: number;
  traceSearchPageSize: number;
  /** R9: every manual compaction call, in order. */
  compactionRequests: string[];
  /**
   * Every transcript read, in order.
   *
   * A stop makes the shell re-read the transcript, and a reader can press send
   * while that read is in flight. The count is what separates "one re-read" from
   * a re-read loop: a loop keeps the composer paused and no submit-during-pause
   * rule can survive it.
   */
  transcriptRequests: string[];
  compactionResult: MockCompactionResult;
  /** R4: every reorder call, in order, with the list it named. */
  reorderRequests: Array<{ sessionId: string; orderedIds: string[] }>;
  /** R6: every fork call, in order, with the cut it named. */
  forkRequests: Array<{ sessionId: string; truncateAfter: number | null }>;
  trustStatuses: Record<string, ProductTrustStatus>;
  trustRequests: Array<{
    method: "GET" | "PUT";
    workspaceId: string;
    body?: string;
  }>;
  /**
   * R8: staged attachment payloads by id, in the order they were uploaded. A
   * reference a message carries is resolved against this, so a test can assert
   * that the transcript renders what the server actually stored.
   */
  attachments: Record<
    string,
    {
      product_session_id: string;
      name: string;
      content_type: string;
      size: number;
      sha256: string;
      bytes: number[];
      warnings: string[];
      status: "staged" | "referenced";
    }
  >;
  /** R8: every upload, in order, with what the request actually carried. */
  attachmentUploads: Array<{
    sessionId: string;
    name: string | null;
    contentType: string;
    bodyBytes: number;
  }>;
  /** R8: every attachment `HEAD` probe, in order. */
  attachmentProbes: Array<{ sessionId: string; attachmentId: string }>;
  /** R8: remaining upload refusals, consumed in order. */
  remainingAttachmentFailures: Array<{ status: number; code: string }>;
  /** F.5: every `POST .../controls/{id}/confirm`, in order, as the URL carried it. */
  followupConfirmRequests: Array<{ sessionId: string; controlId: string }>;
  /** F.5: refusals the confirm route answers before it lets one through. */
  remainingFollowupConfirmFailures: Array<{
    status: number;
    code: string;
    error: string;
  }>;
}

/** R9: the idle answer `POST .../compact` gives, overridable per test. */
interface MockCompactionResult {
  triggered: boolean;
  mode: string;
  degraded: boolean;
  failure_code?: string;
  consecutive_failures: number;
  circuit_open: boolean;
  summary?: string;
  summary_truncated: boolean;
  token_estimate: number;
  source_message_count: number;
}

type MockJobMode =
  | "completed"
  | "approval"
  | "running_tool"
  /** The model is still working: the run is live and has produced nothing yet. */
  | "waiting_model";

interface MockJob {
  jobId: string;
  runId: string;
  resumedFromRunId: string | null;
  sessionId: string;
  message: string;
  mode: MockJobMode;
  status: "running" | "done" | "cancelled";
  events: Array<{ seq: number; event: Record<string, unknown> }>;
}

interface ProviderProfileMutation {
  label: string;
  provider_type: MockProviderProfile["provider_type"];
  api_base: string;
  api_key_env?: string;
  default_model?: string;
  expected_revision?: string;
}

interface DelayedSessionVisibility {
  session: MockSession;
  remainingReads: number;
}

export async function installMockProductApi(
  page: Page,
  options: MockProductApiOptions = {},
): Promise<MockProductApiState> {
  let providerCatalogGeneration = 0;
  const initialProviderCatalogRevision = mockProviderCatalogRevision(
    providerCatalogGeneration,
  );
  const state: MockProductApiState = {
    workspaces: structuredClone(options.workspaces ?? []),
    sessions: structuredClone(options.sessions ?? []),
    sessionModelConfigs: Object.fromEntries(
      (options.sessions ?? []).map((session) => [
        session.id,
        createMockSessionModelConfig(session.id),
      ]),
    ),
    transcripts: structuredClone(options.transcripts ?? {}),
    messages: Object.fromEntries(
      (options.sessions ?? []).map((session) => [session.id, []]),
    ),
    providerProfiles: structuredClone(options.providerProfiles ?? []).map(
      (profile) => ({
        ...profile,
        catalog_revision: initialProviderCatalogRevision,
      }),
    ),
    providerCatalogRevision: initialProviderCatalogRevision,
    providerProfileMutationRequests: [],
    memoryTopics: structuredClone(options.memoryTopics ?? {}),
    memoryWorkspaceRequests: [],
    memoryMutationRequests: 0,
    remainingMemoryMutationFailures: options.memoryMutationFailures ?? 0,
    mcpServers: structuredClone(options.mcpServers ?? {}),
    mcpHealth: structuredClone(options.mcpHealth ?? {}),
    mcpRequestBodies: [],
    mcpMutationRequests: 0,
    remainingMcpMutationFailures: options.mcpMutationFailures ?? 0,
    mcpProbeFailures: structuredClone(options.mcpProbeFailures ?? {}),
    jobs: [],
    jobEffectiveConfigs: [],
    jobStarts: [],
    eventConnections: [],
    preferences: {
      schema_version: 1,
      revision: 0,
      theme: "light",
      default_approval_policy: "ask",
      ...(options.providerSelection
        ? { provider_selection: options.providerSelection }
        : {}),
      ...(options.activeWorkspaceId
        ? { active_workspace_id: options.activeWorkspaceId }
        : {}),
      ...(options.activeSessionId ? { active_session_id: options.activeSessionId } : {}),
    },
    transcriptFailures: { ...(options.transcriptFailures ?? {}) },
    sessionModelConfigUpdateRequests: 0,
    remainingSessionModelConfigFailures:
      options.sessionModelConfigFailures ?? 0,
    sessionModelConfigReads: [],
    remainingSessionModelConfigReadFailures: {
      ...(options.sessionModelConfigReadFailures ?? {}),
    },
    disconnectedJobStartResponses: 0,
    delayedJobBindingReads: 0,
    sessionCreateRequests: 0,
    preferenceUpdateRequests: 0,
    remainingPreferenceUpdateFailures: options.preferenceUpdateFailures ?? 0,
    migrationRequestBodies: [],
    remainingMigrationFailures: options.migrationFailures ?? 0,
    initialStateReadRequests: 0,
    sessionListReads: 0,
    sessionSearchRequests: [],
    searchOnlySessions: structuredClone(options.searchOnlySessions ?? []),
    remainingSessionSearchFailures: options.sessionSearchFailures ?? 0,
    productSearchRequests: [],
    sessionMessageSearchRequests: [],
    searchCorpus: structuredClone(options.searchCorpus ?? {}),
    searchExpiredCursors: [...(options.searchExpiredCursors ?? [])],
    searchExpiredScopes: [...(options.searchExpiredScopes ?? [])],
    remainingProductSearchFailures: options.productSearchFailures ?? 0,
    remainingSessionMessageSearchFailures:
      options.sessionMessageSearchFailures ?? 0,
    traceSearchPageSize: options.traceSearchPageSize ?? 3,
    compactionRequests: [],
    transcriptRequests: [],
    compactionResult: {
      triggered: true,
      mode: "model_generated",
      degraded: false,
      consecutive_failures: 0,
      circuit_open: false,
      summary: "Earlier turns were summarised.",
      summary_truncated: false,
      token_estimate: 128,
      source_message_count: 4,
      ...options.compactionResult,
    },
    reorderRequests: [],
    forkRequests: [],
    trustStatuses: structuredClone(options.trustStatuses ?? {}),
    trustRequests: [],
    attachments: {},
    attachmentUploads: [],
    attachmentProbes: [],
    remainingAttachmentFailures: [...(options.attachmentFailures ?? [])],
    followupConfirmRequests: [],
    remainingFollowupConfirmFailures: [
      ...(options.followupConfirmFailures ?? []),
    ],
  };
  const jobs = new Map<string, MockJob>();
  const delayedSessionVisibility = new Map<string, DelayedSessionVisibility>();
  let workspaceCounter = state.workspaces.length;
  let sessionCounter = state.sessions.length;
  let forkCounter = 0;
  let messageCounter = 0;
  let attachmentCounter = 0;
  let providerProfileCounter = state.providerProfiles.length;
  let migrationReceiptCounter = 0;
  const migratedWorkspaceIds = new Map<string, string>();
  const migratedSessionIds = new Map<string, string>();
  const migratedProfileIds = new Map<string, string>();
  const migrationReceipts = new Map<string, M1BrowserMigrationResponse>();
  const messageIdempotency = new Map<string, ProductMessage>();
  const advanceProviderCatalogRevision = () => {
    providerCatalogGeneration += 1;
    state.providerCatalogRevision = mockProviderCatalogRevision(
      providerCatalogGeneration,
    );
    for (const profile of state.providerProfiles) {
      profile.catalog_revision = state.providerCatalogRevision;
    }
  };

  const startMockJob = (
    body: Record<string, unknown> & {
      message: string;
      product_session_id: string;
    },
    keepActiveUntilObserved = false,
  ) => {
    state.jobs.push(body);
    const session = state.sessions.find(
      (item) => item.id === body.product_session_id,
    );
    if (!session) {
      return {
        ok: false,
        error: {
          status: 404,
          body: { code: "product_not_found", error: "session not found" },
        },
      } as const;
    }
    if (body.resume !== undefined) {
      return {
        ok: false,
        error: {
          status: 409,
          body: {
            code: "product_session_resume_conflict",
            error: "resume must be omitted",
          },
        },
      } as const;
    }

    const modelConfig = state.sessionModelConfigs[session.id]!;
    state.jobEffectiveConfigs.push({
      product_session_id: session.id,
      approval: state.preferences.default_approval_policy as
        | "ask"
        | "auto"
        | "never",
      model: modelConfig.model,
      max_steps: modelConfig.max_steps,
    });
    const sessionBeforeJobStart = structuredClone(session);
    const ordinal = (session.runtime_binding?.ordinal ?? 0) + 1;
    const resumedFromRunId = session.runtime_binding?.latest_run_id ?? null;
    const jobId = session.runtime_binding?.latest_job_id ?? `job-${state.jobs.length}`;
    const runId = `run-${state.jobs.length}`;
    const mode = options.mode ?? "completed";
    const output = outputFor(body.message);
    const events =
      mode === "approval"
        ? approvalEvents(jobId, runId, body.message)
        : mode === "running_tool"
          ? runningToolEvents(jobId, runId, body.message)
          : mode === "waiting_model"
            ? waitingModelEvents(jobId, runId, body.message)
            : completedEvents(jobId, runId, body.message, output);
    const job: MockJob = {
      jobId,
      runId,
      resumedFromRunId,
      sessionId: session.id,
      message: body.message,
      mode,
      status: mode === "completed" ? "done" : "running",
      events,
    };
    jobs.set(jobId, job);
    session.runtime_binding = {
      ordinal,
      runtime_session_id: `runtime-${session.id}`,
      latest_job_id: jobId,
      latest_run_id: runId,
    };
    session.status = keepActiveUntilObserved
      ? "running"
      : mode === "completed"
        ? "idle"
        : "needs_attention";
    session.updated_at = NOW;
    const transcript = state.transcripts[session.id] ?? emptyTranscript(session);
    transcript.segments.push(
      transcriptSegment(
        session,
        ordinal,
        jobId,
        runId,
        resumedFromRunId,
        mode === "completed" ? "done" : "running",
        events,
      ),
    );
    state.transcripts[session.id] = transcript;
    const delayedReads = options.jobBindingVisibilityDelayReads ?? 0;
    if (delayedReads > 0) {
      delayedSessionVisibility.set(session.id, {
        session: sessionBeforeJobStart,
        remainingReads: delayedReads,
      });
    }
    const started = {
      job_id: jobId,
      run_id: runId,
      resumed_from_run_id: resumedFromRunId,
    };
    state.jobStarts.push(started);
    return { ok: true, started, session } as const;
  };

  /**
   * R8: the API decides at read time whether the session's model can take each
   * reference, so the mock stamps the same reason on the same terms.
   */
  const stampAttachmentDegradation = (message: ProductMessage): ProductMessage => {
    const reason = options.attachmentDegradation;
    if (!reason || !message.attachments || message.attachments.length === 0) {
      return message;
    }
    return {
      ...message,
      attachments: message.attachments.map((reference) =>
        reference.availability === "available" &&
        reference.content_type.startsWith("image/")
          ? { ...reference, degradation: reason }
          : reference,
      ),
    };
  };

  const shouldDisconnectJobStart = () => {
    if (
      state.disconnectedJobStartResponses >=
      (options.disconnectJobStartResponses ?? 0)
    ) {
      return false;
    }
    state.disconnectedJobStartResponses += 1;
    return true;
  };

  await page.route(/\/api\/product(?:\/.*)?(?:\?.*)?$/, async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname.replace(/^\/api/u, "");
    const method = request.method();

    if (path === "/product/migrations/m1-browser" && method === "POST") {
      const rawBody = request.postData() ?? "";
      state.migrationRequestBodies.push(rawBody);
      if (state.remainingMigrationFailures > 0) {
        state.remainingMigrationFailures -= 1;
        return json(
          route,
          { code: "product_store_unavailable", error: "migration temporarily unavailable" },
          503,
        );
      }
      const body = request.postDataJSON() as M1BrowserMigrationRequest;
      const existing = migrationReceipts.get(body.idempotency_key);
      if (existing) {
        return json(route, { ...existing, disposition: "already_applied" });
      }

      const workspaceMappings = body.workspaces.map((workspace) => {
        let workspaceId = migratedWorkspaceIds.get(workspace.source_id);
        if (!workspaceId) {
          workspaceCounter += 1;
          workspaceId = `workspace-${workspaceCounter}`;
          migratedWorkspaceIds.set(workspace.source_id, workspaceId);
          state.workspaces.unshift({
            id: workspaceId,
            canonical_root: workspace.root,
            kind: workspace.kind,
            display_name: workspace.display_name,
            pinned: workspace.pinned,
            last_opened_at: workspace.last_opened_at,
            created_at: NOW,
            updated_at: NOW,
          });
        }
        return { source_id: workspace.source_id, workspace_id: workspaceId };
      });
      const sessionMappings = body.sessions.flatMap((session) => {
        const workspaceId = migratedWorkspaceIds.get(session.source_workspace_id);
        if (!workspaceId) {
          return [];
        }
        let sessionId = migratedSessionIds.get(session.source_id);
        if (!sessionId) {
          sessionCounter += 1;
          sessionId = `session-${sessionCounter}`;
          migratedSessionIds.set(session.source_id, sessionId);
          const imported = createMockSession(sessionId, workspaceId, session.title);
          imported.created_at = session.created_at;
          imported.updated_at = session.updated_at;
          state.sessions.unshift(imported);
          state.transcripts[sessionId] = emptyTranscript(imported);
          state.messages[sessionId] = [];
          state.sessionModelConfigs[sessionId] = createMockSessionModelConfig(
            sessionId,
          );
        }
        return [{ source_id: session.source_id, product_session_id: sessionId }];
      });
      let importedProviderProfile = false;
      const providerProfileMappings = body.provider_profiles.map((profile) => {
        let profileId = migratedProfileIds.get(profile.source_id);
        if (!profileId) {
          importedProviderProfile = true;
          providerProfileCounter += 1;
          profileId = `provider-${providerProfileCounter}`;
          migratedProfileIds.set(profile.source_id, profileId);
          state.providerProfiles.unshift({
            id: profileId,
            label: profile.label,
            provider_type: profile.provider_type,
            api_base: profile.api_base,
            created_at: NOW,
            updated_at: profile.updated_at,
            catalog_revision: state.providerCatalogRevision,
            ...(profile.api_key_env ? { api_key_env: profile.api_key_env } : {}),
            ...(profile.default_model ? { default_model: profile.default_model } : {}),
          });
        }
        return { source_id: profile.source_id, provider_profile_id: profileId };
      });
      if (importedProviderProfile) {
        advanceProviderCatalogRevision();
      }

      const importedSelection = body.safe_preferences.provider_selection;
      const mappedProfileId = importedSelection?.source_profile_id
        ? migratedProfileIds.get(importedSelection.source_profile_id)
        : undefined;
      state.preferences = {
        ...state.preferences,
        revision: Number(state.preferences.revision) + 1,
        ...(body.safe_preferences.theme
          ? { theme: body.safe_preferences.theme }
          : {}),
        ...(body.safe_preferences.source_active_workspace_id
          ? {
              active_workspace_id: migratedWorkspaceIds.get(
                body.safe_preferences.source_active_workspace_id,
              ),
            }
          : {}),
        ...(body.safe_preferences.source_active_session_id
          ? {
              active_session_id: migratedSessionIds.get(
                body.safe_preferences.source_active_session_id,
              ),
            }
          : {}),
        ...(importedSelection
          ? {
              provider_selection: {
                ...(mappedProfileId ? { profile_id: mappedProfileId } : {}),
                model: importedSelection.model,
                approval: importedSelection.approval,
                max_steps: importedSelection.max_steps,
              },
            }
          : {}),
      };
      if (importedSelection) {
        for (const session of body.sessions) {
          const mappedSessionId = migratedSessionIds.get(session.source_id);
          if (!mappedSessionId) {
            continue;
          }
          state.sessionModelConfigs[mappedSessionId] = createMockSessionModelConfig(
            mappedSessionId,
            mappedProfileId,
            importedSelection.model,
            importedSelection.max_steps,
          );
        }
      }
      migrationReceiptCounter += 1;
      const acknowledgement: M1BrowserMigrationResponse = {
        source_schema_version: body.source_schema_version,
        idempotency_key: body.idempotency_key,
        receipt_id: `01J00000000000000000000${String(90 + migrationReceiptCounter).padStart(2, "0")}`,
        disposition: "applied",
        workspace_mappings: workspaceMappings,
        session_mappings: sessionMappings,
        provider_profile_mappings: providerProfileMappings,
        issues: options.migrationIssues ?? [],
        applied_at: NOW,
      };
      migrationReceipts.set(body.idempotency_key, acknowledgement);
      return json(route, acknowledgement);
    }

    if (path === "/product/workspaces" && method === "GET") {
      state.initialStateReadRequests += 1;
      return json(route, { workspaces: state.workspaces });
    }

    const trustMatch = path.match(/^\/product\/workspaces\/([^/]+)\/trust$/u);
    if (trustMatch && (method === "GET" || method === "PUT")) {
      const workspaceId = decodeURIComponent(trustMatch[1]!);
      if (!state.workspaces.some((workspace) => workspace.id === workspaceId)) {
        return json(
          route,
          { code: "product_not_found", error: "product workspace was not found" },
          404,
        );
      }
      const current =
        state.trustStatuses[workspaceId] ?? createMockTrustStatus(workspaceId);
      state.trustStatuses[workspaceId] = current;
      if (method === "GET") {
        state.trustRequests.push({ method, workspaceId });
        return json(route, current);
      }
      const rawBody = request.postData() ?? "";
      state.trustRequests.push({ method, workspaceId, body: rawBody });
      const body = request.postDataJSON() as ProductTrustDecisionRequest;
      const capabilities =
        body.capabilities.length > 0
          ? body.capabilities
          : body.decision === "grant"
            ? [...PRODUCT_TRUST_CAPABILITIES]
            : [];
      current.state =
        body.decision === "grant"
          ? "trusted"
          : body.decision === "deny"
            ? "restricted"
            : "revoked";
      current.granted_capabilities =
        body.decision === "grant" ? capabilities : [];
      current.invalidated_capabilities = [];
      return json(route, current);
    }
    if (path === "/product/workspaces" && method === "POST") {
      const body = request.postDataJSON() as {
        root: string;
        kind: "folder" | "repo";
        display_name?: string;
        pinned?: boolean;
      };
      let workspace = state.workspaces.find(
        (item) => item.canonical_root.toLowerCase() === body.root.toLowerCase(),
      );
      if (!workspace) {
        workspaceCounter += 1;
        workspace = createMockWorkspace(
          `workspace-${workspaceCounter}`,
          body.root,
          body.kind,
        );
        state.workspaces.unshift(workspace);
      }
      workspace.kind = body.kind;
      workspace.pinned = body.pinned ?? false;
      workspace.display_name = body.display_name ?? workspace.display_name;
      workspace.updated_at = NOW;
      workspace.last_opened_at = NOW;
      return json(route, workspace, 201);
    }
    const workspaceDelete = path.match(/^\/product\/workspaces\/([^/]+)$/u);
    if (workspaceDelete && method === "DELETE") {
      if (options.workspaceDeleteDelayMs) {
        await new Promise((resolve) =>
          setTimeout(resolve, options.workspaceDeleteDelayMs),
        );
      }
      const workspaceId = decodeURIComponent(workspaceDelete[1]!);
      state.workspaces = state.workspaces.filter((item) => item.id !== workspaceId);
      state.sessions = state.sessions.filter(
        (session) => session.workspace_id !== workspaceId,
      );
      for (const sessionId of Object.keys(state.sessionModelConfigs)) {
        if (!state.sessions.some((session) => session.id === sessionId)) {
          delete state.sessionModelConfigs[sessionId];
          delete state.messages[sessionId];
        }
      }
      delete state.mcpServers[workspaceId];
      delete state.mcpHealth[workspaceId];
      return route.fulfill({ status: 204, body: "" });
    }
    if (path === "/product/sessions" && method === "GET") {
      const workspaceId = url.searchParams.get("workspace_id");
      state.sessionListReads += 1;
      const q = url.searchParams.get("q");
      if (!q) {
        return json(route, {
          sessions: state.sessions
            .filter((session) => session.workspace_id === workspaceId)
            .map((session) => {
              const delayed = delayedSessionVisibility.get(session.id);
              if (!delayed || delayed.remainingReads <= 0) {
                return session;
              }
              delayed.remainingReads -= 1;
              state.delayedJobBindingReads += 1;
              if (delayed.remainingReads === 0) {
                delayedSessionVisibility.delete(session.id);
              }
              return delayed.session;
            }),
        });
      }
      // F11: the server's own search. The fold is deliberately ASCII-only to
      // mirror the SQL `LIKE` the real endpoint uses, which is where it differs
      // from the client's `toLocaleLowerCase`.
      state.sessionSearchRequests.push({
        q,
        limit: url.searchParams.get("limit"),
      });
      if (state.remainingSessionSearchFailures > 0) {
        state.remainingSessionSearchFailures -= 1;
        return json(
          route,
          { code: "product_unavailable", error: "session search is unavailable" },
          503,
        );
      }
      const folded = q.toLowerCase();
      const limit = Number(url.searchParams.get("limit") ?? "200");
      const matched = [...state.sessions, ...state.searchOnlySessions]
        .filter((session) => session.workspace_id === workspaceId)
        .filter((session) => session.title.toLowerCase().includes(folded))
        .slice(0, Number.isFinite(limit) ? limit : 200);
      return json(route, { sessions: matched });
    }
    if (path === "/product/search" && method === "GET") {
      const q = url.searchParams.get("q");
      const rawScope = url.searchParams.get("scope");
      const cursor = url.searchParams.get("cursor");
      const limitRaw = url.searchParams.get("limit");
      state.productSearchRequests.push({
        q,
        scope: rawScope,
        cursor,
        limit: limitRaw,
      });
      const scope = parseProductSearchScope(rawScope);
      if (!scope || searchTermRefusal(q) !== null || searchLimitRefusal(limitRaw)) {
        return json(
          route,
          { code: "product_invalid_input", error: "search input is not usable" },
          400,
        );
      }
      const canonical = `${scope.kind}:${scope.id}`;
      // The route has no trigram path outside the session scope, so a short term
      // in the workspace scope is a refusal rather than an empty page.
      if (scope.kind === "workspace" && [...q!.trim()].length < 3) {
        return json(
          route,
          {
            code: "product_invalid_input",
            error: "the workspace scope needs at least three characters",
          },
          400,
        );
      }
      if (state.remainingProductSearchFailures > 0) {
        state.remainingProductSearchFailures -= 1;
        return json(
          route,
          { code: "product_unavailable", error: "search is unavailable" },
          503,
        );
      }
      let offset = 0;
      if (cursor !== null) {
        if (state.searchExpiredCursors.includes(cursor)) {
          return json(
            route,
            {
              code: "product_events_expired",
              error: "the run this cursor names is no longer here",
            },
            409,
          );
        }
        const parsed = parseSearchCursor(cursor);
        // A cursor is bound to the term *and* the scope that issued it; anything
        // else is the 400 a client gets for reusing one across a changed search.
        if (!parsed || parsed.term !== q || parsed.scope !== canonical) {
          return json(
            route,
            {
              code: "product_invalid_input",
              error: "cursor does not belong to this search",
            },
            400,
          );
        }
        offset = parsed.offset;
      } else if (state.searchExpiredScopes.includes(canonical)) {
        // A first page can expire too: the records the scope names are gone.
        return json(
          route,
          { code: "product_events_expired", error: "the searched records are gone" },
          409,
        );
      }
      const limit = Number(limitRaw ?? "32");
      const hits = state.searchCorpus[canonical] ?? [];
      const page = searchPage({
        hits,
        term: q!,
        scope: canonical,
        cursorOffset: offset,
        // The trace scope shares one byte budget across the runs of a request,
        // so it can answer fewer hits than asked for and still publish a cursor.
        pageSize:
          scope.kind === "trace"
            ? Math.min(limit, state.traceSearchPageSize)
            : limit,
      });
      return json(route, { scope: canonical, ...page });
    }
    const sessionSearchMatch = path.match(
      /^\/product\/sessions\/([^/]+)\/search$/u,
    );
    if (sessionSearchMatch && method === "GET") {
      const sessionId = decodeURIComponent(sessionSearchMatch[1]!);
      const q = url.searchParams.get("q");
      const cursor = url.searchParams.get("cursor");
      const limitRaw = url.searchParams.get("limit");
      state.sessionMessageSearchRequests.push({
        sessionId,
        q,
        cursor,
        limit: limitRaw,
      });
      if (searchTermRefusal(q) !== null || searchLimitRefusal(limitRaw)) {
        return json(
          route,
          { code: "product_invalid_input", error: "search input is not usable" },
          400,
        );
      }
      if (!state.sessions.some((session) => session.id === sessionId)) {
        return json(
          route,
          { code: "product_not_found", error: "session not found" },
          404,
        );
      }
      if (state.remainingSessionMessageSearchFailures > 0) {
        state.remainingSessionMessageSearchFailures -= 1;
        return json(
          route,
          { code: "product_unavailable", error: "search is unavailable" },
          503,
        );
      }
      const canonical = `session:${sessionId}`;
      let offset = 0;
      if (cursor !== null) {
        const parsed = parseSearchCursor(cursor);
        if (!parsed || parsed.term !== q || parsed.scope !== canonical) {
          return json(
            route,
            {
              code: "product_invalid_input",
              error: "cursor does not belong to this search",
            },
            400,
          );
        }
        offset = parsed.offset;
      }
      const page = searchPage({
        hits: state.searchCorpus[canonical] ?? [],
        term: q!,
        scope: canonical,
        cursorOffset: offset,
        pageSize: Number(limitRaw ?? "32"),
      });
      return json(route, {
        hits: page.hits.map((hit): ProductMessageSearchHit => ({
          message_seq: hit.seq,
          snippet: hit.snippet,
          created_at: hit.created_at ?? NOW,
        })),
        ...(page.next_cursor ? { next_cursor: page.next_cursor } : {}),
      });
    }
    const compactionMatch = path.match(
      /^\/product\/sessions\/([^/]+)\/compact$/u,
    );
    if (compactionMatch && method === "POST") {
      const sessionId = decodeURIComponent(compactionMatch[1]!);
      const session = state.sessions.find((item) => item.id === sessionId);
      if (!session) {
        return json(
          route,
          { code: "product_not_found", error: "session not found" },
          404,
        );
      }
      state.compactionRequests.push(sessionId);
      // The route takes the same exclusive per-session claim a turn takes, so a
      // live run is refused instead of being queued behind it.
      const activeJob = session.runtime_binding?.latest_job_id
        ? jobs.get(session.runtime_binding.latest_job_id)
        : undefined;
      if (activeJob?.status === "running") {
        return json(
          route,
          { code: "product_session_active", error: "session is still running" },
          409,
        );
      }
      return json(route, {
        product_session_id: sessionId,
        ...(session.runtime_binding
          ? { runtime_run_id: session.runtime_binding.latest_run_id }
          : {}),
        ...state.compactionResult,
      });
    }
    const reorderMatch = path.match(
      /^\/product\/sessions\/([^/]+)\/messages\/reorder$/u,
    );
    if (reorderMatch && method === "POST") {
      const sessionId = decodeURIComponent(reorderMatch[1]!);
      if (!state.sessions.some((session) => session.id === sessionId)) {
        return json(
          route,
          { code: "product_not_found", error: "session not found" },
          404,
        );
      }
      const body = request.postDataJSON() as { ordered_ids?: unknown };
      const orderedIds = body?.ordered_ids;
      // Shape first, the way the client parser and the server both read it: a
      // bounded list of unique ids. Anything else is malformed input, not a
      // conflicting queue.
      if (
        !Array.isArray(orderedIds) ||
        orderedIds.length === 0 ||
        orderedIds.length > 64 ||
        orderedIds.some((id) => typeof id !== "string" || id.length === 0) ||
        new Set(orderedIds).size !== orderedIds.length
      ) {
        return json(
          route,
          {
            code: "product_invalid_input",
            error: "ordered_ids must be a bounded list of unique ids",
          },
          400,
        );
      }
      const named = orderedIds as string[];
      state.reorderRequests.push({ sessionId, orderedIds: named });
      // Exact coverage: the list must name *exactly* the session's pending
      // successor queue. A partial or stale list is refused instead of merged,
      // which is what turns a concurrent promote, revoke, or drain into a typed
      // conflict.
      const queued = (state.messages[sessionId] ?? []).filter(
        (message) =>
          message.status === "queued" &&
          message.requested_delivery === "successor",
      );
      const queuedIds = new Set(queued.map((message) => message.id));
      if (
        named.length !== queuedIds.size ||
        named.some((id) => !queuedIds.has(id))
      ) {
        return json(
          route,
          {
            code: "product_control_conflict",
            error: "ordered_ids must name exactly the pending successor queue",
          },
          409,
        );
      }
      named.forEach((id, index) => {
        const message = queued.find((item) => item.id === id);
        if (message) {
          message.queue_order = index + 1;
        }
      });
      return json(route, {
        messages: (state.messages[sessionId] ?? [])
          .filter(
            (message) =>
              message.status === "queued" ||
              message.status === "needs_attention",
          )
          .sort(
            (left, right) =>
              (left.queue_order ?? left.seq) - (right.queue_order ?? right.seq),
          ),
      });
    }
    const forkMatch = path.match(/^\/product\/sessions\/([^/]+)\/forks$/u);
    if (forkMatch && method === "POST") {
      const parentId = decodeURIComponent(forkMatch[1]!);
      const parent = state.sessions.find((session) => session.id === parentId);
      if (!parent?.runtime_binding) {
        return json(
          route,
          { code: "product_conflict", error: "session cannot be forked" },
          409,
        );
      }
      const body = request.postDataJSON() as {
        fork_at_run_id?: string;
        truncate_after_message_seq?: unknown;
        idempotency_key?: string;
      };
      const forkRunId = body.fork_at_run_id ?? parent.runtime_binding.latest_run_id;
      // The route takes the same exclusive per-session claim a turn takes, so a
      // live parent is refused rather than forked behind its running turn.
      const parentJob = parent.runtime_binding.latest_job_id
        ? jobs.get(parent.runtime_binding.latest_job_id)
        : undefined;
      if (parentJob?.status === "running") {
        return json(
          route,
          { code: "product_session_active", error: "parent session is still running" },
          409,
        );
      }
      // R6: the optional cut must name a ledger sequence this fork's run
      // delivered. A sequence that is not a positive integer, or that no ledger
      // row holds, is malformed input; one held by a *different* run of the same
      // session is a source conflict — neither is silently ignored.
      let truncateAfter: number | null = null;
      if (body.truncate_after_message_seq !== undefined) {
        const raw = body.truncate_after_message_seq;
        if (typeof raw !== "number" || !Number.isInteger(raw) || raw <= 0) {
          return json(
            route,
            {
              code: "product_invalid_input",
              error: "truncate_after_message_seq must be a positive ledger sequence",
            },
            400,
          );
        }
        const target = (state.messages[parentId] ?? []).find(
          (message) => message.seq === raw,
        );
        if (!target) {
          return json(
            route,
            {
              code: "product_invalid_input",
              error: "truncate_after_message_seq is not in the parent ledger",
            },
            400,
          );
        }
        if (target.run_id !== forkRunId) {
          return json(
            route,
            {
              code: "product_fork_source_invalid",
              error: "truncate_after_message_seq belongs to another run",
            },
            409,
          );
        }
        truncateAfter = raw;
      }
      state.forkRequests.push({ sessionId: parentId, truncateAfter });
      if (options.forkRefusal) {
        return json(
          route,
          { code: options.forkRefusal.code, error: "the cut cannot be honoured" },
          options.forkRefusal.status,
        );
      }
      forkCounter += 1;
      const childId = `session-fork-${forkCounter}`;
      const child: MockSession = {
        ...createMockSession(childId, parent.workspace_id, `Fork of ${parent.title}`),
        parent_session_id: parentId,
        fork_point_run_id: forkRunId,
        fork_point_seq: truncateAfter ?? 1,
      };
      state.sessions.unshift(child);
      return json(
        route,
        {
          fork: {
            id: `fork-${forkCounter}`,
            parent_product_session_id: parentId,
            child_product_session_id: childId,
            parent_workspace_id: parent.workspace_id,
            parent_title: parent.title,
            source_runtime_session_id: parent.runtime_binding.runtime_session_id,
            source_runtime_job_id: parent.runtime_binding.latest_job_id,
            source_runtime_run_id: parent.runtime_binding.latest_run_id,
            ...(truncateAfter === null
              ? {}
              : { truncate_after_message_seq: truncateAfter }),
            fork_at_event_seq: 1,
            idempotency_key: body.idempotency_key ?? `fork-key-${forkCounter}`,
            created_at: NOW,
          },
          session: child,
        },
        201,
      );
    }
    if (path === "/product/sessions" && method === "POST") {
      state.sessionCreateRequests += 1;
      if (options.sessionCreateDelayMs) {
        await new Promise((resolve) =>
          setTimeout(resolve, options.sessionCreateDelayMs),
        );
      }
      const body = request.postDataJSON() as { workspace_id: string; title?: string };
      sessionCounter += 1;
      const session = createMockSession(
        `session-${sessionCounter}`,
        body.workspace_id,
        body.title,
      );
      state.sessions.unshift(session);
      state.transcripts[session.id] = emptyTranscript(session);
      state.messages[session.id] = [];
      const selection = state.preferences.provider_selection as
        | { profile_id?: string; model?: string; max_steps?: number }
        | undefined;
      state.sessionModelConfigs[session.id] = createMockSessionModelConfig(
        session.id,
        selection?.profile_id,
        selection?.model ?? "fake",
        selection?.max_steps ?? 8,
      );
      return json(route, session, 201);
    }
    const messageActionMatch = path.match(
      /^\/product\/sessions\/([^/]+)\/messages\/([^/]+)\/(promote|revoke)$/u,
    );
    if (messageActionMatch && method === "POST") {
      const sessionId = decodeURIComponent(messageActionMatch[1]!);
      const messageId = decodeURIComponent(messageActionMatch[2]!);
      const action = messageActionMatch[3]!;
      const message = (state.messages[sessionId] ?? []).find(
        (item) => item.id === messageId,
      );
      if (!message) {
        return json(
          route,
          { code: "product_not_found", error: "message not found" },
          404,
        );
      }
      if (action === "promote") {
        const activeJob = message.run_id
          ? [...jobs.values()].find(
              (job) => job.runId === message.run_id && job.status === "running",
            )
          : undefined;
        if (message.status !== "queued" || !activeJob) {
          return json(
            route,
            {
              code: "product_control_rejected",
              error: "message is not eligible for promotion",
            },
            409,
          );
        }
        message.requested_delivery = "current_run";
        message.status = "intervention_requested";
      } else {
        if (message.status !== "queued" && message.status !== "needs_attention") {
          return json(
            route,
            {
              code: "product_control_rejected",
              error: "message is not eligible for revoke",
            },
            409,
          );
        }
        message.status = "revoked";
      }
      return json(route, message);
    }
    const confirmMatch = path.match(
      /^\/product\/sessions\/([^/]+)\/controls\/([^/]+)\/confirm$/u,
    );
    if (confirmMatch && method === "POST") {
      const sessionId = decodeURIComponent(confirmMatch[1]!);
      const controlId = decodeURIComponent(confirmMatch[2]!);
      state.followupConfirmRequests.push({ sessionId, controlId });
      const message = (state.messages[sessionId] ?? []).find(
        (item) => item.id === controlId,
      );
      if (!message) {
        return json(
          route,
          { code: "product_not_found", error: "message not found" },
          404,
        );
      }
      // A message and its control share one id, so the ledger row is the control
      // the route reads. The store's `ProductControlStatus` + `requested_delivery`
      // -> `ProductMessageStatus` mapping is inverted here for the two statuses
      // the confirm route can find a control already in without re-queueing it;
      // everything else is the abandoned row this route exists to recover.
      const alreadyWaiting =
        message.status === "queued" ||
        message.status === "intervention_requested" ||
        message.status === "claimed_successor" ||
        message.status === "applied_current_run";
      const control = (status: string) => ({
        id: message.id,
        product_session_id: sessionId,
        kind: message.requested_delivery === "successor" ? "followup" : "steer",
        content: message.content,
        status,
        seq: message.seq,
        created_at: message.created_at,
      });
      const failure = state.remainingFollowupConfirmFailures.shift();
      if (failure) {
        return json(
          route,
          { code: failure.code, error: failure.error },
          failure.status,
        );
      }
      if (
        [...jobs.values()].some(
          (job) => job.sessionId === sessionId && job.status === "running",
        )
      ) {
        return json(
          route,
          {
            code: "product_session_active",
            error:
              "an abandoned follow-up cannot be confirmed while a turn is active",
          },
          409,
        );
      }
      if (message.requested_delivery !== "successor") {
        return json(
          route,
          {
            code: "product_control_rejected",
            error: "only follow-up controls can be confirmed",
          },
          409,
        );
      }
      // The route returns a control it already holds `pending`/`accepted`/
      // `applied` unchanged, so a repeat is idempotent instead of a second
      // re-queue.
      if (alreadyWaiting) {
        return json(
          route,
          control(
            message.status === "queued" ||
              message.status === "intervention_requested"
              ? "pending"
              : "applied",
          ),
        );
      }
      if (message.status !== "needs_attention") {
        return json(
          route,
          {
            code: "product_control_rejected",
            error: "only an abandoned follow-up can be confirmed",
          },
          409,
        );
      }
      if (options.followupConfirmDelayMs) {
        // Held open before anything moves: the request is genuinely in flight and
        // the row still says `needs_attention`.
        await new Promise((resolve) =>
          setTimeout(resolve, options.followupConfirmDelayMs),
        );
      }
      // The committed transition: the control returns to `pending` (a successor
      // waiting again), the session leaves `needs_attention`, and the server's
      // abandoned reason goes with the abandoned row. The re-drain the real
      // route then attempts is API behaviour with its own API tests; this mock
      // answers exactly what the confirm response commits.
      const confirmed = control("pending");
      message.status = "queued";
      delete message.reason;
      const session = state.sessions.find((item) => item.id === sessionId);
      if (session) {
        session.status = "idle";
      }
      return json(route, confirmed);
    }
    const attachmentsMatch = path.match(
      /^\/product\/sessions\/([^/]+)\/attachments$/u,
    );
    if (attachmentsMatch && method === "POST") {
      const sessionId = decodeURIComponent(attachmentsMatch[1]!);
      if (!state.sessions.some((session) => session.id === sessionId)) {
        return json(
          route,
          { code: "product_not_found", error: "session not found" },
          404,
        );
      }
      const rawBody = request.postDataBuffer();
      state.attachmentUploads.push({
        sessionId,
        name: url.searchParams.get("name"),
        contentType: request.headers()["content-type"] ?? "",
        bodyBytes: rawBody?.byteLength ?? 0,
      });
      const failure = state.remainingAttachmentFailures.shift();
      if (failure) {
        return json(
          route,
          { code: failure.code, error: `attachment upload refused: ${failure.code}` },
          failure.status,
        );
      }
      attachmentCounter += 1;
      const attachmentId = `attachment-${attachmentCounter}`;
      const name = url.searchParams.get("name") ?? "attachment";
      // The real route verifies the bytes and stores the sniffed type; the mock
      // records the claim, which is what a client can be held to.
      const stored = {
        product_session_id: sessionId,
        name,
        content_type: request.headers()["content-type"] ?? "application/octet-stream",
        size: rawBody?.byteLength ?? 0,
        sha256: "d".repeat(64),
        bytes: [...(rawBody ?? new Uint8Array())],
        warnings: [...(options.attachmentWarnings ?? [])],
        status: "staged" as const,
      };
      state.attachments[attachmentId] = stored;
      if (options.attachmentUploadBody !== undefined) {
        return json(route, options.attachmentUploadBody, 201);
      }
      return json(
        route,
        {
          attachment_id: attachmentId,
          product_session_id: sessionId,
          content_type: stored.content_type,
          size: stored.size,
          sha256: stored.sha256,
          name: stored.name,
          status: stored.status,
          warnings: stored.warnings,
        },
        201,
      );
    }
    const attachmentMatch = path.match(
      /^\/product\/sessions\/([^/]+)\/attachments\/([^/]+)$/u,
    );
    if (attachmentMatch && (method === "HEAD" || method === "GET")) {
      const sessionId = decodeURIComponent(attachmentMatch[1]!);
      const attachmentId = decodeURIComponent(attachmentMatch[2]!);
      state.attachmentProbes.push({ sessionId, attachmentId });
      const failure = options.attachmentProbeFailures?.[attachmentId]?.shift();
      if (failure) {
        return json(
          route,
          { code: failure.code, error: `attachment refused: ${failure.code}` },
          failure.status,
        );
      }
      const attachment = state.attachments[attachmentId];
      if (!attachment || attachment.product_session_id !== sessionId) {
        return json(
          route,
          { code: "product_attachment_not_found", error: "attachment not found" },
          404,
        );
      }
      return route.fulfill({
        status: 200,
        headers: {
          "content-type": attachment.content_type,
          "content-disposition": `attachment; filename="${attachment.name}"`,
        },
        // A HEAD probe answers the headers and nothing else.
        ...(method === "HEAD" ? {} : { body: Buffer.from(attachment.bytes) }),
      });
    }
    const messagesMatch = path.match(
      /^\/product\/sessions\/([^/]+)\/messages$/u,
    );
    if (messagesMatch && method === "GET") {
      const sessionId = decodeURIComponent(messagesMatch[1]!);
      if (!state.sessions.some((session) => session.id === sessionId)) {
        return json(
          route,
          { code: "product_not_found", error: "session not found" },
          404,
        );
      }
      const all = state.messages[sessionId] ?? [];
      const limit = Math.max(1, Math.min(128, Number(url.searchParams.get("limit") ?? 64)));
      const after = url.searchParams.get("after_seq");
      const before = url.searchParams.get("before_seq");
      if (after !== null) {
        const eligible = all.filter((message) => message.seq > Number(after));
        const messages = eligible.slice(0, limit).map(stampAttachmentDegradation);
        return json(route, {
          messages,
          ...(eligible.length > limit && messages.length > 0
            ? { next_after_seq: messages.at(-1)!.seq }
            : {}),
        });
      }
      const boundary = before === null ? Number.POSITIVE_INFINITY : Number(before);
      const eligible = all.filter((message) => message.seq < boundary);
      const first = Math.max(0, eligible.length - limit);
      const messages = eligible.slice(first).map(stampAttachmentDegradation);
      return json(route, {
        messages,
        ...(first > 0 && messages.length > 0
          ? { next_before_seq: messages[0]!.seq }
          : {}),
      });
    }
    if (messagesMatch && method === "POST") {
      const sessionId = decodeURIComponent(messagesMatch[1]!);
      const session = state.sessions.find((item) => item.id === sessionId);
      if (!session) {
        return json(
          route,
          { code: "product_not_found", error: "session not found" },
          404,
        );
      }
      const body = request.postDataJSON() as {
        content: string;
        idempotency_key?: string;
        attachments?: Array<{ attachment_id: string; name?: string }>;
      };
      const content = body.content?.trim();
      if (!content) {
        return json(
          route,
          { code: "product_invalid_input", error: "message is empty" },
          400,
        );
      }
      // R8: a reference the session does not own is refused, exactly as the
      // real route refuses it, instead of being stored as a dead reference.
      const references: NonNullable<ProductMessage["attachments"]> = [];
      for (const requested of body.attachments ?? []) {
        const stored = state.attachments[requested.attachment_id];
        if (!stored || stored.product_session_id !== sessionId) {
          return json(
            route,
            {
              code: "product_attachment_not_found",
              error: "attachment not found for this session",
            },
            404,
          );
        }
        stored.status = "referenced";
        references.push({
          attachment_id: requested.attachment_id,
          content_type: stored.content_type,
          size: stored.size,
          sha256: stored.sha256,
          name: requested.name ?? stored.name,
          availability: "available",
        });
      }
      const idempotencyKey = body.idempotency_key
        ? `${sessionId}\u0000${body.idempotency_key}`
        : undefined;
      const replayed = idempotencyKey
        ? messageIdempotency.get(idempotencyKey)
        : undefined;
      if (replayed) {
        if (replayed.content !== content) {
          return json(
            route,
            {
              code: "product_idempotency_conflict",
              error: "idempotency key already has different content",
            },
            409,
          );
        }
        return json(route, replayed);
      }

      messageCounter += 1;
      const messages = (state.messages[sessionId] ??= []);
      const activeJobId = session.runtime_binding?.latest_job_id;
      const activeJob = activeJobId ? jobs.get(activeJobId) : undefined;
      const message: ProductMessage = {
        id: `message-${messageCounter}`,
        product_session_id: sessionId,
        content,
        requested_delivery: "successor",
        status: activeJob?.status === "running" ? "queued" : "claimed_successor",
        seq: (messages.at(-1)?.seq ?? 0) + 1,
        created_at: NOW,
        ...(references.length > 0 ? { attachments: references } : {}),
        ...(activeJob?.status === "running"
          ? { run_id: activeJob.runId }
          : {}),
      };
      const responseMessage = structuredClone(message);
      if (message.status === "claimed_successor") {
        responseMessage.status = "queued";
      }
      messages.push(message);
      if (idempotencyKey) {
        messageIdempotency.set(idempotencyKey, message);
      }

      if (message.status === "claimed_successor") {
        const workspace = state.workspaces.find(
          (item) => item.id === session.workspace_id,
        );
        if (!workspace) {
          message.status = "needs_attention";
          message.reason = "workspace is unavailable";
        } else {
          const started = startMockJob(
            {
              message: content,
              product_session_id: sessionId,
              workspace: {
                kind: workspace.kind,
                root: workspace.canonical_root,
              },
            },
            true,
          );
          if (!started.ok) {
            message.status = "needs_attention";
            message.reason = started.error.body.error;
          } else {
            message.actual_delivery = "successor";
            message.successor_run_id = started.started.run_id;
          }
        }
      }
      if (shouldDisconnectJobStart()) {
        return route.abort("connectionreset");
      }
      return json(route, responseMessage, 201);
    }
    const transcriptMatch = path.match(
      /^\/product\/sessions\/([^/]+)\/transcript$/u,
    );
    if (transcriptMatch && method === "GET") {
      const sessionId = decodeURIComponent(transcriptMatch[1]!);
      state.transcriptRequests.push(sessionId);
      const remainingFailures = state.transcriptFailures[sessionId] ?? 0;
      if (remainingFailures > 0) {
        state.transcriptFailures[sessionId] = remainingFailures - 1;
        return json(
          route,
          { code: "product_storage_failure", error: "transcript store unavailable" },
          503,
        );
      }
      const delay = options.transcriptDelayMs?.[sessionId] ?? 0;
      if (delay > 0) {
        await new Promise((resolve) => setTimeout(resolve, delay));
      }
      const session = state.sessions.find((item) => item.id === sessionId);
      if (!session) {
        return json(route, { code: "product_not_found", error: "session not found" }, 404);
      }
      const page = transcriptPage(
        state.transcripts[sessionId] ?? emptyTranscript(session),
        url.searchParams,
      );
      if ("error" in page) {
        return json(
          route,
          { code: "product_invalid_input", error: page.error },
          400,
        );
      }
      return json(route, page);
    }
    const sessionModelConfigMatch = path.match(
      /^\/product\/sessions\/([^/]+)\/model-config$/u,
    );
    if (sessionModelConfigMatch) {
      const sessionId = decodeURIComponent(sessionModelConfigMatch[1]!);
      const config = state.sessionModelConfigs[sessionId];
      if (!config || !state.sessions.some((session) => session.id === sessionId)) {
        return json(
          route,
          { code: "product_not_found", error: "session model config not found" },
          404,
        );
      }
      if (method === "GET") {
        state.sessionModelConfigReads.push(sessionId);
        if ((state.remainingSessionModelConfigReadFailures[sessionId] ?? 0) > 0) {
          state.remainingSessionModelConfigReadFailures[sessionId] -= 1;
          return json(
            route,
            {
              code: "product_storage_failure",
              error: "session model settings unavailable",
            },
            503,
          );
        }
        return json(route, config);
      }
      if (method === "PUT") {
        state.sessionModelConfigUpdateRequests += 1;
        if (state.remainingSessionModelConfigFailures > 0) {
          state.remainingSessionModelConfigFailures -= 1;
          return json(
            route,
            {
              code: "product_storage_failure",
              error: "session model settings unavailable",
            },
            503,
          );
        }
        const body = request.postDataJSON() as {
          profile_id?: string;
          model: string;
          reasoning?: "default" | "low" | "medium" | "high";
          max_steps?: number;
          expected_revision?: number;
        };
        if (
          body.expected_revision !== undefined &&
          body.expected_revision !== config.revision
        ) {
          return json(
            route,
            {
              code: "product_session_model_config_conflict",
              error: "session model config revision does not match",
            },
            409,
          );
        }
        if (
          body.profile_id &&
          !state.providerProfiles.some((profile) => profile.id === body.profile_id)
        ) {
          return json(
            route,
            {
              code: "product_provider_profile_unavailable",
              error: "provider profile not found",
            },
            404,
          );
        }
        config.profile_id = body.profile_id;
        config.model = body.model;
        config.reasoning = body.reasoning ?? "default";
        config.max_steps = body.max_steps ?? 8;
        config.revision += 1;
        config.updated_at = NOW;
        return json(route, config);
      }
    }
    const sessionRunModelsMatch = path.match(
      /^\/product\/sessions\/([^/]+)\/run-models$/u,
    );
    if (sessionRunModelsMatch && method === "GET") {
      const sessionId = decodeURIComponent(sessionRunModelsMatch[1]!);
      if (!state.sessions.some((session) => session.id === sessionId)) {
        return json(
          route,
          { code: "product_not_found", error: "session not found" },
          404,
        );
      }
      return json(route, { runs: [] });
    }
    const sessionUsageMatch = path.match(
      /^\/product\/sessions\/([^/]+)\/usage$/u,
    );
    if (sessionUsageMatch && method === "GET") {
      const sessionId = decodeURIComponent(sessionUsageMatch[1]!);
      if (!state.sessions.some((session) => session.id === sessionId)) {
        return json(
          route,
          { code: "product_not_found", error: "session not found" },
          404,
        );
      }
      return json(route, {
        product_session_id: sessionId,
        totals: {
          prompt_tokens: 0,
          completion_tokens: 0,
          total_tokens: 0,
          cached_tokens: 0,
        },
        totals_cost: {
          currency: "USD",
          availability: "local_zero",
          total_usd: 0,
          prompt_usd: 0,
          completion_usd: 0,
          cache_read_usd: 0,
          pricing_source: "bundled",
          pricing_version: "mock",
        },
        runs: [],
        partial_reasons: [],
      });
    }
    const sessionExportMatch = path.match(
      /^\/product\/sessions\/([^/]+)\/export$/u,
    );
    if (sessionExportMatch && method === "POST") {
      const sessionId = decodeURIComponent(sessionExportMatch[1]!);
      const session = state.sessions.find((item) => item.id === sessionId);
      if (!session) {
        return json(
          route,
          { code: "product_not_found", error: "session not found" },
          404,
        );
      }
      const format = url.searchParams.get("format") ?? "json";
      const mediaType =
        format === "html"
          ? "text/html; charset=utf-8"
          : format === "markdown"
            ? "text/markdown; charset=utf-8"
            : "application/json; charset=utf-8";
      const extension = format === "markdown" ? "md" : format;
      const body =
        format === "html"
          ? `<!doctype html><html><body><h1>${session.title}</h1></body></html>`
          : format === "markdown"
            ? `# ${session.title}\n`
            : JSON.stringify({
                schema_version: 1,
                product_session_id: session.id,
                title: session.title,
                transcript: state.transcripts[session.id] ?? emptyTranscript(session),
              });
      return route.fulfill({
        status: 200,
        headers: {
          "content-type": mediaType,
          "content-disposition": `attachment; filename="rove-session-${session.id}-evidence.${extension}"`,
        },
        body,
      });
    }
    const sessionMatch = path.match(/^\/product\/sessions\/([^/]+)$/u);
    if (sessionMatch && method === "PATCH") {
      const sessionId = decodeURIComponent(sessionMatch[1]!);
      const session = state.sessions.find((item) => item.id === sessionId);
      if (!session) {
        return json(route, { code: "product_not_found", error: "session not found" }, 404);
      }
      const body = request.postDataJSON() as { title?: string; archived?: boolean };
      if (body.title) {
        session.title = body.title;
      }
      if (body.archived !== undefined) {
        session.status = body.archived ? "archived" : "idle";
      }
      session.updated_at = NOW;
      return json(route, session);
    }
    if (sessionMatch && method === "DELETE") {
      const sessionId = decodeURIComponent(sessionMatch[1]!);
      state.sessions = state.sessions.filter((item) => item.id !== sessionId);
      delete state.transcripts[sessionId];
      delete state.messages[sessionId];
      delete state.sessionModelConfigs[sessionId];
      return route.fulfill({ status: 204, body: "" });
    }
    if (path === "/product/preferences" && method === "GET") {
      state.initialStateReadRequests += 1;
      return json(route, state.preferences);
    }
    if (path === "/product/preferences" && method === "PUT") {
      state.preferenceUpdateRequests += 1;
      if (options.preferenceUpdateDelayMs) {
        await new Promise((resolve) =>
          setTimeout(resolve, options.preferenceUpdateDelayMs),
        );
      }
      if (state.remainingPreferenceUpdateFailures > 0) {
        state.remainingPreferenceUpdateFailures -= 1;
        return json(
          route,
          { code: "product_storage_failure", error: "preferences unavailable" },
          503,
        );
      }
      const body = request.postDataJSON() as Record<string, unknown>;
      const {
        expected_revision: expectedRevision,
        provider_selection: requestedSelection,
        ...requestedPreferences
      } = body;
      const currentRevision = Number(state.preferences.revision);
      if (expectedRevision !== currentRevision) {
        return json(
          route,
          {
            code: "product_revision_conflict",
            error: "preferences revision does not match",
          },
          409,
        );
      }
      const defaultApproval = requestedPreferences.default_approval_policy;
      const providerSelection =
        requestedSelection && typeof requestedSelection === "object"
          ? {
              ...(requestedSelection as Record<string, unknown>),
              approval: defaultApproval,
            }
          : undefined;
      state.preferences = {
        ...requestedPreferences,
        revision: currentRevision + 1,
        ...(providerSelection ? { provider_selection: providerSelection } : {}),
      };
      return json(route, state.preferences);
    }
    if (path === "/product/memory/topics" && method === "GET") {
      const workspaceId = url.searchParams.get("workspace_id");
      state.memoryWorkspaceRequests.push(workspaceId);
      if (!state.workspaces.some((workspace) => workspace.id === workspaceId)) {
        return json(
          route,
          { code: "product_not_found", error: "product workspace was not found" },
          404,
        );
      }
      const q = url.searchParams.get("q")?.toLocaleLowerCase();
      const memoryType = url.searchParams.get("memory_type");
      const scope = url.searchParams.get("scope");
      const source = url.searchParams.get("source");
      const topics = Object.values(state.memoryTopics)
        .map((entry) => entry.topic)
        .filter(
          (topic) =>
            (q === undefined ||
              topic.slug.toLocaleLowerCase().includes(q) ||
              topic.title.toLocaleLowerCase().includes(q) ||
              topic.description.toLocaleLowerCase().includes(q)) &&
            (memoryType === null || topic.memory_type === memoryType) &&
            (scope === null || topic.scope === scope) &&
            (source === null || topic.source === source),
        );
      return json(route, { topics, total: topics.length });
    }
    if (path === "/product/memory/topics" && method === "POST") {
      const workspaceId = url.searchParams.get("workspace_id");
      state.memoryWorkspaceRequests.push(workspaceId);
      if (!state.workspaces.some((workspace) => workspace.id === workspaceId)) {
        return json(
          route,
          { code: "product_not_found", error: "product workspace was not found" },
          404,
        );
      }
      state.memoryMutationRequests += 1;
      if (state.remainingMemoryMutationFailures > 0) {
        state.remainingMemoryMutationFailures -= 1;
        return json(
          route,
          {
            code: "product_memory_conflict",
            error: "memory topic changed concurrently",
          },
          409,
        );
      }
      const body = request.postDataJSON() as CreateProductMemoryTopicRequest;
      if (state.memoryTopics[body.slug] !== undefined) {
        return json(
          route,
          {
            code: "product_memory_conflict",
            error: "memory topic already exists",
          },
          409,
        );
      }
      const timestamp = new Date(
        Date.parse(NOW) + state.memoryMutationRequests * 1_000,
      ).toISOString();
      const created: ProductMemoryTopicContentResponse = {
        topic: {
          slug: body.slug,
          title: body.title,
          layer: "durable",
          memory_type: body.memory_type,
          scope: body.scope,
          source: "product_settings",
          confidence: body.confidence,
          created_at: timestamp,
          updated_at: timestamp,
          description: body.description,
          metadata_truncated: false,
        },
        content: body.content,
        truncated: false,
      };
      state.memoryTopics[body.slug] = created;
      return json(route, created, 201);
    }
    const memoryTopicMatch = path.match(
      /^\/product\/memory\/topics\/([^/]+)$/u,
    );
    if (memoryTopicMatch && method === "PUT") {
      const workspaceId = url.searchParams.get("workspace_id");
      state.memoryWorkspaceRequests.push(workspaceId);
      if (!state.workspaces.some((workspace) => workspace.id === workspaceId)) {
        return json(
          route,
          { code: "product_not_found", error: "product workspace was not found" },
          404,
        );
      }
      const slug = decodeURIComponent(memoryTopicMatch[1]!);
      const current = state.memoryTopics[slug];
      if (current === undefined) {
        return json(
          route,
          { code: "product_memory_not_found", error: "memory topic not found" },
          404,
        );
      }
      state.memoryMutationRequests += 1;
      if (state.remainingMemoryMutationFailures > 0) {
        state.remainingMemoryMutationFailures -= 1;
        return json(
          route,
          {
            code: "product_memory_conflict",
            error: "memory topic changed concurrently",
          },
          409,
        );
      }
      const body = request.postDataJSON() as UpdateProductMemoryTopicRequest;
      if (body.expected_updated_at !== current.topic.updated_at) {
        return json(
          route,
          {
            code: "product_memory_conflict",
            error: "memory topic changed concurrently",
          },
          409,
        );
      }
      const updated: ProductMemoryTopicContentResponse = {
        topic: {
          ...current.topic,
          title: body.title,
          memory_type: body.memory_type,
          scope: body.scope,
          source: "product_settings",
          confidence: body.confidence,
          updated_at: new Date(
            Date.parse(NOW) + state.memoryMutationRequests * 1_000,
          ).toISOString(),
          description: body.description,
        },
        content: body.content,
        truncated: false,
      };
      state.memoryTopics[slug] = updated;
      return json(route, updated);
    }
    if (memoryTopicMatch && method === "GET") {
      const workspaceId = url.searchParams.get("workspace_id");
      state.memoryWorkspaceRequests.push(workspaceId);
      if (!state.workspaces.some((workspace) => workspace.id === workspaceId)) {
        return json(
          route,
          { code: "product_not_found", error: "product workspace was not found" },
          404,
        );
      }
      const slug = decodeURIComponent(memoryTopicMatch[1]!);
      const topic = state.memoryTopics[slug];
      return topic
        ? json(route, topic)
        : json(
            route,
            { code: "product_not_found", error: "memory topic not found" },
            404,
          );
    }
    if (memoryTopicMatch && method === "DELETE") {
      const workspaceId = url.searchParams.get("workspace_id");
      state.memoryWorkspaceRequests.push(workspaceId);
      if (!state.workspaces.some((workspace) => workspace.id === workspaceId)) {
        return json(
          route,
          { code: "product_not_found", error: "product workspace was not found" },
          404,
        );
      }
      const slug = decodeURIComponent(memoryTopicMatch[1]!);
      if (!state.memoryTopics[slug]) {
        return json(
          route,
          { code: "product_not_found", error: "memory topic not found" },
          404,
        );
      }
      delete state.memoryTopics[slug];
      return route.fulfill({ status: 204, body: "" });
    }
    if (path === "/product/mcp/health" && method === "GET") {
      const workspaceId = url.searchParams.get("workspace_id");
      if (
        workspaceId === null ||
        !state.workspaces.some((workspace) => workspace.id === workspaceId)
      ) {
        return json(
          route,
          { code: "product_not_found", error: "product workspace was not found" },
          404,
        );
      }
      const cached = new Map(
        (state.mcpHealth[workspaceId] ?? []).map((entry) => [
          entry.server_name,
          entry,
        ]),
      );
      const servers = (state.mcpServers[workspaceId] ?? []).map((server) =>
        cached.get(server.name) ?? {
          server_name: server.name,
          required: server.required,
          transport: server.transport,
          status: server.enabled ? "unknown" : "disabled",
          tool_count: 0,
        },
      );
      return json(route, { servers, total: servers.length });
    }
    if (path === "/product/mcp/servers") {
      const workspaceId = url.searchParams.get("workspace_id");
      if (
        workspaceId === null ||
        !state.workspaces.some((workspace) => workspace.id === workspaceId)
      ) {
        return json(
          route,
          { code: "product_not_found", error: "product workspace was not found" },
          404,
        );
      }
      const servers = state.mcpServers[workspaceId] ?? [];
      if (method === "GET") {
        return json(route, { servers, total: servers.length });
      }
      if (method === "POST") {
        state.mcpRequestBodies.push(request.postData() ?? "");
        state.mcpMutationRequests += 1;
        if (state.remainingMcpMutationFailures > 0) {
          state.remainingMcpMutationFailures -= 1;
          return json(
            route,
            {
              code: "product_mcp_conflict",
              error: "MCP config is locked by another settings operation",
            },
            409,
          );
        }
        const body = request.postDataJSON() as CreateProductMcpServerRequest;
        if (servers.some((server) => server.name === body.name)) {
          return json(
            route,
            { code: "product_mcp_conflict", error: "MCP server already exists" },
            409,
          );
        }
        const created: ProductMcpServerConfig = {
          ...structuredClone(body),
          transport_deprecated: body.transport === "sse",
        };
        state.mcpServers[workspaceId] = [...servers, created].sort((left, right) =>
          left.name.localeCompare(right.name),
        );
        state.mcpHealth[workspaceId] = (state.mcpHealth[workspaceId] ?? []).filter(
          (entry) => entry.server_name !== created.name,
        );
        return json(route, created, 201);
      }
    }
    const mcpProbeMatch = path.match(
      /^\/product\/mcp\/servers\/([^/]+)\/probe$/u,
    );
    if (mcpProbeMatch && method === "POST") {
      const workspaceId = url.searchParams.get("workspace_id");
      if (
        workspaceId === null ||
        !state.workspaces.some((workspace) => workspace.id === workspaceId)
      ) {
        return json(
          route,
          { code: "product_not_found", error: "product workspace was not found" },
          404,
        );
      }
      const name = decodeURIComponent(mcpProbeMatch[1]!);
      const server = (state.mcpServers[workspaceId] ?? []).find(
        (candidate) => candidate.name === name,
      );
      if (!server) {
        return json(
          route,
          { code: "product_mcp_not_found", error: "MCP server was not found" },
          404,
        );
      }
      const scopedFailureKey = `${workspaceId}:${name}`;
      const failureQueue =
        state.mcpProbeFailures[scopedFailureKey] ?? state.mcpProbeFailures[name];
      const failure = failureQueue?.shift();
      if (failure) {
        return json(
          route,
          { code: failure.code, error: failure.error },
          failure.status,
        );
      }
      const tools = structuredClone(
        options.mcpProbeTools ?? createMockMcpProbeTools(name),
      );
      return json(route, {
        server_name: name,
        transport: server.transport,
        tools,
        tested_at: NOW,
      });
    }
    const mcpServerMatch = path.match(/^\/product\/mcp\/servers\/([^/]+)$/u);
    if (mcpServerMatch) {
      const workspaceId = url.searchParams.get("workspace_id");
      if (
        workspaceId === null ||
        !state.workspaces.some((workspace) => workspace.id === workspaceId)
      ) {
        return json(
          route,
          { code: "product_not_found", error: "product workspace was not found" },
          404,
        );
      }
      const name = decodeURIComponent(mcpServerMatch[1]!);
      const servers = state.mcpServers[workspaceId] ?? [];
      const serverIndex = servers.findIndex((server) => server.name === name);
      if (serverIndex < 0) {
        return json(
          route,
          { code: "product_mcp_not_found", error: "MCP server was not found" },
          404,
        );
      }
      if (method === "PUT") {
        state.mcpRequestBodies.push(request.postData() ?? "");
        state.mcpMutationRequests += 1;
        if (state.remainingMcpMutationFailures > 0) {
          state.remainingMcpMutationFailures -= 1;
          return json(
            route,
            {
              code: "product_mcp_conflict",
              error: "MCP config is locked by another settings operation",
            },
            409,
          );
        }
        const body = request.postDataJSON() as UpdateProductMcpServerRequest;
        const updated: ProductMcpServerConfig = {
          name,
          ...body,
          transport_deprecated: body.transport === "sse",
        };
        servers[serverIndex] = updated;
        state.mcpServers[workspaceId] = [...servers].sort((left, right) =>
          left.name.localeCompare(right.name),
        );
        state.mcpHealth[workspaceId] = (state.mcpHealth[workspaceId] ?? []).filter(
          (entry) => entry.server_name !== name,
        );
        return json(route, updated);
      }
      if (method === "DELETE") {
        state.mcpMutationRequests += 1;
        if (state.remainingMcpMutationFailures > 0) {
          state.remainingMcpMutationFailures -= 1;
          return json(
            route,
            {
              code: "product_mcp_conflict",
              error: "MCP config is locked by another settings operation",
            },
            409,
          );
        }
        state.mcpServers[workspaceId] = servers.filter(
          (server) => server.name !== name,
        );
        state.mcpHealth[workspaceId] = (state.mcpHealth[workspaceId] ?? []).filter(
          (entry) => entry.server_name !== name,
        );
        return route.fulfill({ status: 204, body: "" });
      }
    }
    if (path === "/product/runtime" && method === "GET") {
      const needsAttentionCount = state.sessions.filter(
        (session) => session.status === "needs_attention",
      ).length;
      return json(route, {
        api_version: "0.1.0",
        connection: "connected",
        product_store: "ready",
        execution_environment: {
          adapter: "local",
          workspace_kind: "folder",
          workspace_digest:
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
          capabilities: {
            filesystem_read: true,
            filesystem_write: true,
            process_run: true,
            process_stdio: true,
            observations: true,
            process_background: true,
            process_pty: false,
            workspace_checkpoints: true,
            artifact_projection: true,
          },
        },
        agent: {
          selector: "builtin:legacy",
          workspace_source_authorized: false,
          workspace_instructions_enabled: false,
          allow_remediation_procedures: false,
          max_procedure_selections: 3,
        },
        resume_health: {
          status: needsAttentionCount === 0 ? "healthy" : "needs_attention",
          workspace_count: state.workspaces.length,
          session_count: state.sessions.length,
          bound_session_count: state.sessions.filter(
            (session) => session.runtime_binding !== undefined,
          ).length,
          running_session_count: state.sessions.filter(
            (session) => session.status === "running",
          ).length,
          needs_attention_session_count: needsAttentionCount,
        },
      });
    }
    if (path === "/product/provider-profiles" && method === "GET") {
      state.initialStateReadRequests += 1;
      return json(route, {
        catalog_revision: state.providerCatalogRevision,
        provider_profiles: state.providerProfiles,
      });
    }
    if (path === "/product/provider-profiles" && method === "POST") {
      const body = request.postDataJSON() as ProviderProfileMutation;
      state.providerProfileMutationRequests.push({
        method: "POST",
        ...(body.expected_revision
          ? { expectedRevision: body.expected_revision }
          : {}),
      });
      if (
        body.expected_revision !== undefined &&
        body.expected_revision !== state.providerCatalogRevision
      ) {
        return providerCatalogConflict(route);
      }
      providerProfileCounter += 1;
      const profile: MockProviderProfile = {
        id: `provider-${providerProfileCounter}`,
        label: body.label,
        provider_type: body.provider_type,
        api_base: body.api_base,
        created_at: NOW,
        updated_at: NOW,
        catalog_revision: state.providerCatalogRevision,
        ...(body.api_key_env ? { api_key_env: body.api_key_env } : {}),
        ...(body.default_model ? { default_model: body.default_model } : {}),
      };
      state.providerProfiles.unshift(profile);
      advanceProviderCatalogRevision();
      return json(route, profile, 201);
    }
    const providerModelsMatch = path.match(
      /^\/product\/provider-profiles\/([^/]+)\/models$/u,
    );
    if (providerModelsMatch && method === "GET") {
      const profileId = decodeURIComponent(providerModelsMatch[1]!);
      const profile = state.providerProfiles.find((item) => item.id === profileId);
      if (!profile) {
        return json(
          route,
          { code: "product_not_found", error: "provider profile not found" },
          404,
        );
      }
      const supportsReasoning = profile.provider_type === "openai-responses";
      return json(route, {
        profile_id: profile.id,
        ...(profile.default_model ? { default_model: profile.default_model } : {}),
        models: profile.default_model
          ? [
              {
                id: profile.default_model,
                supports_reasoning: supportsReasoning,
                supported_reasoning: supportsReasoning
                  ? ["low", "medium", "high"]
                  : [],
                ...(supportsReasoning
                  ? {}
                  : {
                      reasoning_unavailable_reason:
                        "Reasoning controls are only available for OpenAI Responses profiles.",
                    }),
              },
            ]
          : [],
      });
    }
    const providerProfileMatch = path.match(
      /^\/product\/provider-profiles\/([^/]+)$/u,
    );
    if (providerProfileMatch && method === "PUT") {
      const profileId = decodeURIComponent(providerProfileMatch[1]!);
      const profile = state.providerProfiles.find((item) => item.id === profileId);
      if (!profile) {
        return json(
          route,
          { code: "product_not_found", error: "provider profile not found" },
          404,
        );
      }
      const body = request.postDataJSON() as ProviderProfileMutation;
      state.providerProfileMutationRequests.push({
        method: "PUT",
        profileId,
        ...(body.expected_revision
          ? { expectedRevision: body.expected_revision }
          : {}),
      });
      if (
        body.expected_revision !== undefined &&
        body.expected_revision !== state.providerCatalogRevision
      ) {
        return providerCatalogConflict(route);
      }
      profile.label = body.label;
      profile.provider_type = body.provider_type;
      profile.api_base = body.api_base;
      profile.updated_at = NOW;
      replaceOptional(profile, "api_key_env", body.api_key_env);
      replaceOptional(profile, "default_model", body.default_model);
      advanceProviderCatalogRevision();
      return json(route, profile);
    }
    if (providerProfileMatch && method === "DELETE") {
      const profileId = decodeURIComponent(providerProfileMatch[1]!);
      const expectedRevision = url.searchParams.get("expected_revision") ?? undefined;
      state.providerProfileMutationRequests.push({
        method: "DELETE",
        profileId,
        ...(expectedRevision ? { expectedRevision } : {}),
      });
      const profileIndex = state.providerProfiles.findIndex(
        (item) => item.id === profileId,
      );
      if (profileIndex < 0) {
        return json(
          route,
          { code: "product_not_found", error: "provider profile not found" },
          404,
        );
      }
      if (
        expectedRevision !== undefined &&
        expectedRevision !== state.providerCatalogRevision
      ) {
        return providerCatalogConflict(route);
      }
      state.providerProfiles.splice(profileIndex, 1);
      for (const config of Object.values(state.sessionModelConfigs)) {
        if (config.profile_id === profileId) {
          delete config.profile_id;
          config.revision += 1;
          config.updated_at = NOW;
        }
      }
      advanceProviderCatalogRevision();
      return route.fulfill({ status: 204, body: "" });
    }
    return json(route, { code: "product_not_found", error: `unmocked ${method} ${path}` }, 404);
  });

  await page.route("/api/jobs", async (route) => {
    const body = route.request().postDataJSON() as Record<string, unknown> & {
      message: string;
      product_session_id: string;
    };
    const result = startMockJob(body);
    if (!result.ok) {
      return json(route, result.error.body, result.error.status);
    }
    if (shouldDisconnectJobStart()) {
      return route.abort("connectionreset");
    }
    return json(route, result.started);
  });

  await page.route(/\/api\/jobs\/[^/]+\/events$/u, async (route) => {
    const job = jobFromRoute(route, jobs);
    if (!job) {
      return route.fulfill({ status: 404, body: "" });
    }
    const session = state.sessions.find((item) => item.id === job.sessionId);
    if (session) {
      session.status = job.status === "running" ? "needs_attention" : "idle";
      session.updated_at = NOW;
    }
    state.eventConnections.push(job.jobId);
    return route.fulfill({
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "cache-control": "no-cache",
      },
      body: job.events.map((stored) => sse(stored.event.type as string, stored.seq, stored.event)).join(""),
    });
  });

  await page.route(/\/api\/jobs\/[^/]+\/state$/u, async (route) => {
    const job = jobFromRoute(route, jobs);
    if (!job) {
      return route.fulfill({ status: 404, body: "" });
    }
    const session = state.sessions.find((item) => item.id === job.sessionId);
    if (session) {
      session.status = job.status === "running" ? "needs_attention" : "idle";
      session.updated_at = NOW;
    }
    return fulfillJobState(route, job);
  });

  await page.route(/\/api\/jobs\/[^/]+\/approvals\/[^/]+$/u, async (route) => {
    const job = jobFromRoute(route, jobs);
    if (!job) {
      return route.fulfill({ status: 404, body: "" });
    }
    const session = state.sessions.find((item) => item.id === job.sessionId)!;
    const segment = state.transcripts[job.sessionId]!.segments.at(-1)! as {
      events: Array<{ seq: number; event: Record<string, unknown> }>;
      observed_through_seq: number;
      last_event_seq: number;
      run_status: string;
    };
    const completed = approvalCompletionEvents();
    // segment.events and job.events alias one array (transcriptSegment stores
    // the creation events by reference), so pushing twice would duplicate the
    // completion events and the restore parser would rightly reject the
    // non-contiguous seqs.
    job.events.push(...completed);
    segment.observed_through_seq = job.events.at(-1)!.seq;
    segment.last_event_seq = job.events.at(-1)!.seq;
    segment.run_status = "done";
    job.status = "done";
    session.status = "idle";
    return fulfillJobState(route, job, completed);
  });

  await page.route(/\/api\/jobs\/[^/]+\/cancel$/u, async (route) => {
    const job = jobFromRoute(route, jobs);
    if (!job) {
      return route.fulfill({ status: 404, body: "" });
    }
    job.status = "cancelled";
    const session = state.sessions.find((item) => item.id === job.sessionId);
    if (session) {
      session.status = "idle";
    }
    if (options.salvageOnCancel !== undefined) {
      // The salvage is the tail of the run: a terminal `llm_message` marked
      // `aborted` and carrying what the model had already produced. Appending it
      // keeps the sequence contiguous, which is what the restore parser requires.
      const seq = (job.events.at(-1)?.seq ?? 0) + 1;
      job.events.push({
        seq,
        event: {
          type: "llm_message",
          full: options.salvageOnCancel,
          aborted: true,
          usage: { prompt_tokens: 4, completion_tokens: 3, total_tokens: 7 },
        },
      });
      const segment = state.transcripts[job.sessionId]?.segments.at(-1) as
        | {
            events: Array<{ seq: number; event: Record<string, unknown> }>;
            observed_through_seq: number;
            last_event_seq: number;
            run_status: string;
          }
        | undefined;
      if (segment) {
        segment.observed_through_seq = seq;
        segment.last_event_seq = seq;
        segment.run_status = "done";
      }
    }
    return fulfillJobState(route, job);
  });

  await page.route(/\/api\/jobs\/[^/]+\/inputs\/[^/]+$/u, async (route) => {
    const job = jobFromRoute(route, jobs);
    return job ? fulfillJobState(route, job) : route.fulfill({ status: 404, body: "" });
  });

  await page.route(/\/api\/runs(?:\?.*)?$/u, async (route) =>
    json(route, { runs: [] }),
  );

  return state;
}

export function createMockWorkspace(
  id = "workspace-1",
  root = "D:/tmp/rove-shell-demo",
  kind: "folder" | "repo" = "folder",
): MockWorkspace {
  return {
    id,
    canonical_root: root,
    kind,
    display_name: root.split(/[\\/]/u).filter(Boolean).at(-1) ?? "workspace",
    pinned: false,
    last_opened_at: NOW,
    created_at: NOW,
    updated_at: NOW,
  };
}

function createMockTrustStatus(workspaceId: string): ProductTrustStatus {
  return {
    workspace_id: workspaceId,
    state: "unknown",
    identity_digest:
      "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    invalidated_capabilities: [],
    granted_capabilities: [],
  };
}

export function createMockSession(
  id = "session-1",
  workspaceId = "workspace-1",
  title = "Durable session",
  lastOutcome?: MockSession["last_outcome"],
): MockSession {
  return {
    id,
    workspace_id: workspaceId,
    title,
    status: "idle",
    created_at: NOW,
    updated_at: NOW,
    ...(lastOutcome === undefined
      ? {}
      : { last_outcome: lastOutcome, last_outcome_at: NOW }),
  };
}

export function createMockSessionModelConfig(
  sessionId: string,
  profileId?: string,
  model = "fake",
  maxSteps = 8,
): MockSessionModelConfig {
  return {
    product_session_id: sessionId,
    ...(profileId ? { profile_id: profileId } : {}),
    model,
    reasoning: "default",
    max_steps: maxSteps,
    revision: 1,
    updated_at: NOW,
  };
}

function createMockMcpProbeTools(serverName: string): ProductMcpToolDescriptor[] {
  return [
    {
      name: `mcp__${serverName}__echo_remote`,
      description: "Echo a value through the configured MCP server",
      destructive: true,
      parallel_safe: false,
    },
    {
      name: `mcp__${serverName}__workspace_status`,
      description: "Read the remote workspace status",
      destructive: true,
      parallel_safe: false,
    },
  ];
}

export function completedTranscript(
  workspace: MockWorkspace,
  session: MockSession,
  question = "Restored question",
  answer = "Restored answer",
): MockTranscript {
  const jobId = "job-restored-1";
  const runId = "run-restored-1";
  const events = completedEvents(jobId, runId, question, answer);
  session.runtime_binding = {
    ordinal: 1,
    runtime_session_id: `runtime-${session.id}`,
    latest_job_id: jobId,
    latest_run_id: runId,
  };
  return {
    product_session_id: session.id,
    workspace_id: workspace.id,
    status: "complete",
    partial_reasons: [],
    segments: [
      transcriptSegment(session, 1, jobId, runId, null, "done", events),
    ],
  };
}

/**
 * A completed session whose durable row at `withheldSeq` was withheld from this
 * client.
 *
 * The server knew the row's kind; the `event_contract` this bundle declared did
 * not, so the row is absent from the delivered events while its durable position
 * is reported as an `unknown_event_type` reason. The response is a real partial
 * restore, not a failure: the surrounding events still arrive.
 */
export function unknownEventTranscript(
  workspace: MockWorkspace,
  session: MockSession,
  withheldSeq: number,
): MockTranscript {
  const transcript = completedTranscript(workspace, session);
  const segment = transcript.segments[0]!;
  const events = (segment.events as Array<{ seq: number }>).filter(
    (event) => event.seq !== withheldSeq,
  );
  const binding = segment.binding as { runtime_run_id: string };
  return {
    ...transcript,
    status: "partial",
    partial_reasons: [
      {
        code: "unknown_event_type",
        run_ordinal: 1,
        run_id: binding.runtime_run_id,
        expected_seq: withheldSeq,
        observed_seq: withheldSeq,
      },
    ],
    segments: [{ ...segment, events }],
  };
}

/**
 * A completed session with `runCount` contiguous runs. Long enough that the
 * API's 64-run page ceiling spans several cursor pages.
 */
export function longCompletedTranscript(
  workspace: MockWorkspace,
  session: MockSession,
  runCount: number,
): MockTranscript {
  const segments = Array.from({ length: runCount }, (_value, index) => {
    const ordinal = index + 1;
    const jobId = `job-restored-${ordinal}`;
    const runId = `run-restored-${ordinal}`;
    return transcriptSegment(
      session,
      ordinal,
      jobId,
      runId,
      ordinal > 1 ? `run-restored-${ordinal - 1}` : null,
      "done",
      completedEvents(
        jobId,
        runId,
        `Restored question ${ordinal}`,
        `Restored answer ${ordinal}`,
      ),
    );
  });
  session.runtime_binding = {
    ordinal: runCount,
    runtime_session_id: `runtime-${session.id}`,
    latest_job_id: `job-restored-${runCount}`,
    latest_run_id: `run-restored-${runCount}`,
  };
  return {
    product_session_id: session.id,
    workspace_id: workspace.id,
    status: "complete",
    partial_reasons: [],
    segments,
  };
}

function emptyTranscript(session: MockSession): MockTranscript {
  return {
    product_session_id: session.id,
    workspace_id: session.workspace_id,
    status: "complete",
    partial_reasons: [],
    segments: [],
  };
}

/** Longest run page the API serves; mirrors `limit_runs`' documented range. */
const MAX_MOCK_TRANSCRIPT_RUNS = 64;

/** Page size when only `before_ordinal` is sent; the documented default. */
const DEFAULT_MOCK_TRANSCRIPT_RUNS = 256;

function segmentOrdinal(segment: Record<string, unknown>): number {
  const binding = segment.binding as { ordinal?: unknown } | undefined;
  return typeof binding?.ordinal === "number" ? binding.ordinal : 0;
}

function positiveInteger(value: string | null): number | null {
  if (value === null || !/^\d+$/u.test(value)) {
    return null;
  }
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) && parsed > 0 ? parsed : null;
}

/**
 * R7 search helpers, shared by the session-scoped and the unified route.
 *
 * The mock refuses what the real route refuses — a missing, blank, over-long or
 * control-bearing term, a limit outside 1..100, a scope that is not
 * `workspace:`/`session:`/`trace:`, and a cursor that does not belong to this
 * exact term and scope are all 400 — because a permissive mock would let the Web
 * half ship a request the API answers with an error.
 */
function searchTermRefusal(q: string | null): "missing" | "unusable" | null {
  if (q === null || q.trim() === "") {
    return "missing";
  }
  if (new TextEncoder().encode(q).length > 128) {
    return "unusable";
  }
  return /[\u0000-\u001f\u007f-\u009f]/u.test(q) ? "unusable" : null;
}

function searchLimitRefusal(limit: string | null): boolean {
  if (limit === null) {
    return false;
  }
  const parsed = Number(limit);
  return !Number.isInteger(parsed) || parsed < 1 || parsed > 100;
}

/** A cursor carries the term and the scope it was issued for, like a digest. */
function searchCursorId(term: string, scope: string, offset: number): string {
  return `mock-cursor:${encodeURIComponent(term)}:${encodeURIComponent(scope)}:${offset}`;
}

function parseSearchCursor(
  value: string,
): { term: string; scope: string; offset: number } | null {
  const match = /^mock-cursor:([^:]*):([^:]*):(\d+)$/u.exec(value);
  if (!match) {
    return null;
  }
  return {
    term: decodeURIComponent(match[1]!),
    scope: decodeURIComponent(match[2]!),
    offset: Number(match[3]!),
  };
}

function parseProductSearchScope(
  value: string | null,
): { kind: "workspace" | "session" | "trace"; id: string } | null {
  if (value === null) {
    return null;
  }
  const separator = value.indexOf(":");
  if (separator <= 0) {
    return null;
  }
  const kind = value.slice(0, separator);
  const id = value.slice(separator + 1);
  if (kind !== "workspace" && kind !== "session" && kind !== "trace") {
    return null;
  }
  if (id === "" || /[\u0000-\u001f\u007f-\u009f:\s]/u.test(id)) {
    return null;
  }
  return { kind, id };
}

function searchPage(input: {
  hits: ProductSearchHit[];
  term: string;
  scope: string;
  cursorOffset: number;
  pageSize: number;
}): { hits: ProductSearchHit[]; next_cursor?: string } {
  const slice = input.hits.slice(
    input.cursorOffset,
    input.cursorOffset + input.pageSize,
  );
  const nextOffset = input.cursorOffset + slice.length;
  return {
    hits: slice,
    ...(nextOffset < input.hits.length
      ? { next_cursor: searchCursorId(input.term, input.scope, nextOffset) }
      : {}),
  };
}

/**
 * Cursor pagination for the transcript route.
 *
 * A request without either cursor parameter stays the legacy response: every
 * run, and no `has_more`/`next_before_ordinal` keys. A request with at least one
 * cursor parameter returns a newest-to-oldest page of whole runs plus those keys,
 * and out-of-range input is a typed 400 exactly as the API answers.
 *
 * `event_contract` is validated like the API validates it but changes nothing
 * here: these fixtures carry only kinds the current bundle decodes, so the server
 * has nothing to withhold. `unknownEventTranscript` below is the fixture that
 * exercises the withheld-row shape.
 */
function transcriptPage(
  transcript: MockTranscript,
  params: URLSearchParams,
): MockTranscript | { error: string } {
  const contractRaw = params.get("event_contract");
  if (contractRaw !== null && positiveInteger(contractRaw) === null) {
    return { error: "event_contract must be a positive integer contract version" };
  }
  const beforeRaw = params.get("before_ordinal");
  const limitRaw = params.get("limit_runs");
  if (beforeRaw === null && limitRaw === null) {
    return {
      ...transcript,
      segments: [...transcript.segments].sort(
        (left, right) => segmentOrdinal(left) - segmentOrdinal(right),
      ),
    };
  }
  let beforeOrdinal: number | null = null;
  if (beforeRaw !== null) {
    beforeOrdinal = positiveInteger(beforeRaw);
    if (beforeOrdinal === null) {
      return { error: "before_ordinal must be a positive integer" };
    }
  }
  let limitRuns = DEFAULT_MOCK_TRANSCRIPT_RUNS;
  if (limitRaw !== null) {
    const parsed = positiveInteger(limitRaw);
    if (parsed === null || parsed > MAX_MOCK_TRANSCRIPT_RUNS) {
      return { error: `limit_runs must be between 1 and ${MAX_MOCK_TRANSCRIPT_RUNS}` };
    }
    limitRuns = parsed;
  }
  const ordinals = transcript.segments.map(segmentOrdinal);
  const newestOrdinal = ordinals.length === 0 ? null : Math.max(...ordinals);
  if (
    beforeOrdinal !== null &&
    (newestOrdinal === null || beforeOrdinal > newestOrdinal)
  ) {
    // A cursor past the newest run selects nothing: "older than this" has no
    // answer, and clamping it to the newest page would hand the client runs it
    // already holds.
    return {
      ...transcript,
      status: "complete",
      partial_reasons: [],
      segments: [],
      has_more: false,
    };
  }
  const older = [...transcript.segments]
    .filter(
      (segment) =>
        beforeOrdinal === null || segmentOrdinal(segment) < beforeOrdinal,
    )
    .sort((left, right) => segmentOrdinal(right) - segmentOrdinal(left));
  const page = older.slice(0, limitRuns).reverse();
  const hasMore = older.length > page.length;
  const pageOrdinals = new Set(page.map(segmentOrdinal));
  const reasons = transcript.partial_reasons.filter(
    (reason) =>
      typeof reason.run_ordinal !== "number" || pageOrdinals.has(reason.run_ordinal),
  );
  return {
    ...transcript,
    status: reasons.length === 0 ? "complete" : "partial",
    partial_reasons: reasons,
    segments: page,
    has_more: hasMore,
    ...(hasMore ? { next_before_ordinal: segmentOrdinal(page[0]!) } : {}),
  };
}

function transcriptSegment(
  session: MockSession,
  ordinal: number,
  jobId: string,
  runId: string,
  resumedFromRunId: string | null,
  runStatus: "running" | "done",
  events: Array<{ seq: number; event: Record<string, unknown> }>,
) {
  return {
    binding: {
      product_session_id: session.id,
      ordinal,
      runtime_session_id: `runtime-${session.id}`,
      runtime_job_id: jobId,
      runtime_run_id: runId,
      ...(resumedFromRunId ? { resumed_from_run_id: resumedFromRunId } : {}),
      bound_at: NOW,
    },
    inherited: false,
    run_status: runStatus,
    observed_through_seq: events.at(-1)?.seq ?? 0,
    last_event_seq: events.at(-1)?.seq ?? 0,
    events,
  };
}

function completedEvents(
  jobId: string,
  runId: string,
  message: string,
  output: string,
) {
  return [
    {
      seq: 1,
      event: { type: "run_started", job_id: jobId, run_id: runId, user_message: message },
    },
    { seq: 2, event: { type: "llm_chunk", delta: output } },
    {
      seq: 3,
      event: {
        type: "llm_message",
        full: output,
        usage: { prompt_tokens: 4, completion_tokens: 3, total_tokens: 7 },
      },
    },
    { seq: 4, event: { type: "run_completed", reason: "final", output } },
  ];
}

/**
 * A job whose first tool call never completes within the test's window, so the
 * activity group stays in flight and its elapsed value keeps ticking.
 */
function runningToolEvents(jobId: string, runId: string, message: string) {
  return [
    {
      seq: 1,
      event: { type: "run_started", job_id: jobId, run_id: runId, user_message: message },
    },
    {
      seq: 2,
      event: {
        type: "tool_call_started",
        call_id: "call-running-1",
        name: "shell",
        args: { command: "cargo test --workspace" },
      },
    },
  ];
}

/**
 * A run that is still waiting on the model: it started and has produced nothing
 * yet, so a stop has something to put back (design F6's smart-stop gate).
 */
function waitingModelEvents(jobId: string, runId: string, message: string) {
  return [
    {
      seq: 1,
      event: { type: "run_started", job_id: jobId, run_id: runId, user_message: message },
    },
  ];
}

function approvalEvents(jobId: string, runId: string, message: string) {
  return [
    {
      seq: 1,
      event: { type: "run_started", job_id: jobId, run_id: runId, user_message: message },
    },
    {
      seq: 2,
      event: {
        type: "tool_call_approval_needed",
        call_id: "call-approval-1",
        name: "write_file",
        args: { path: "notes.md" },
        reason: "destructive tool requires explicit approval",
      },
    },
  ];
}

function approvalCompletionEvents() {
  return [
    {
      seq: 3,
      event: {
        type: "tool_call_completed",
        call_id: "call-approval-1",
        result: {
          call_id: "call-approval-1",
          output: "Approved write completed",
          metadata: {
            status: "ok",
            risk_level: "high",
            read_only: false,
            affected_paths: ["notes.md"],
            workspace_changed: true,
            diff_summary: [],
          },
        },
      },
    },
    {
      seq: 4,
      event: {
        type: "run_completed",
        reason: "final",
        output: "Approved write completed",
      },
    },
  ];
}

function outputFor(message: string): string {
  if (/first turn/iu.test(message)) {
    return "First turn done";
  }
  if (/second turn/iu.test(message)) {
    return "Second turn done";
  }
  return "Runtime summary complete";
}

function jobFromRoute(route: Route, jobs: Map<string, MockJob>): MockJob | undefined {
  const match = new URL(route.request().url()).pathname.match(/\/jobs\/([^/]+)/u);
  return match ? jobs.get(decodeURIComponent(match[1]!)) : undefined;
}

function replaceOptional(
  profile: MockProviderProfile,
  key: "api_key_env" | "default_model",
  value: string | undefined,
) {
  if (value) {
    profile[key] = value;
  } else {
    delete profile[key];
  }
}

function mockProviderCatalogRevision(generation: number): string {
  return `sha256:mock-provider-catalog-${generation}`;
}

function providerCatalogConflict(route: Route) {
  return json(
    route,
    {
      code: "product_revision_conflict",
      error: "provider catalog revision does not match",
    },
    409,
  );
}

async function fulfillJobState(
  route: Route,
  job: MockJob,
  events = job.events,
) {
  const approvalPending = job.mode === "approval" && job.status === "running";
  return json(route, {
    job_id: job.jobId,
    run_id: job.runId,
    resumed_from_run_id: job.resumedFromRunId,
    status: job.status,
    event_count: job.events.length,
    events,
    pending_approvals: approvalPending
      ? [
          {
            call_id: "call-approval-1",
            name: "write_file",
            args: { path: "notes.md" },
            reason: "destructive tool requires explicit approval",
          },
        ]
      : [],
    pending_inputs: [],
  });
}

function sse(event: string, id: number, data: unknown): string {
  return `id: ${id}\nevent: ${event}\ndata: ${JSON.stringify(data)}\n\n`;
}

async function json(route: Route, body: unknown, status = 200) {
  return route.fulfill({
    status,
    contentType: "application/json",
    body: JSON.stringify(body),
  });
}
