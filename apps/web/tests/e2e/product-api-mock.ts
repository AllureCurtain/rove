import type { Page, Route } from "@playwright/test";

import type {
  CreateProductMemoryTopicRequest,
  CreateProductMcpServerRequest,
  ProductMemoryTopicContentResponse,
  ProductMcpServerConfig,
  ProductMcpToolDescriptor,
  UpdateProductMcpServerRequest,
  UpdateProductMemoryTopicRequest,
} from "../../settings/settings-platform-api-types";
import type {
  M1BrowserMigrationRequest,
  M1BrowserMigrationResponse,
  M1MigrationIssue,
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
}

export interface MockTranscript {
  product_session_id: string;
  workspace_id: string;
  status: "complete" | "partial";
  partial_reasons: Array<Record<string, unknown>>;
  segments: Array<Record<string, unknown>>;
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
}

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
  providerProfiles?: MockProviderProfile[];
  memoryTopics?: Record<string, ProductMemoryTopicContentResponse>;
  memoryMutationFailures?: number;
  mcpServers?: Record<string, ProductMcpServerConfig[]>;
  mcpMutationFailures?: number;
  mcpProbeFailures?: Record<string, MockMcpProbeFailure[]>;
  mcpProbeTools?: ProductMcpToolDescriptor[];
  activeWorkspaceId?: string;
  activeSessionId?: string;
  mode?: "completed" | "approval";
  transcriptDelayMs?: Record<string, number>;
  transcriptFailures?: Record<string, number>;
  sessionModelConfigFailures?: number;
  disconnectJobStartResponses?: number;
  jobBindingVisibilityDelayReads?: number;
  sessionCreateDelayMs?: number;
  workspaceDeleteDelayMs?: number;
  preferenceUpdateFailures?: number;
  preferenceUpdateDelayMs?: number;
  migrationFailures?: number;
  migrationIssues?: M1MigrationIssue[];
}

export interface MockProductApiState {
  workspaces: MockWorkspace[];
  sessions: MockSession[];
  sessionModelConfigs: Record<string, MockSessionModelConfig>;
  transcripts: Record<string, MockTranscript>;
  providerProfiles: MockProviderProfile[];
  memoryTopics: Record<string, ProductMemoryTopicContentResponse>;
  memoryWorkspaceRequests: Array<string | null>;
  memoryMutationRequests: number;
  remainingMemoryMutationFailures: number;
  mcpServers: Record<string, ProductMcpServerConfig[]>;
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
  disconnectedJobStartResponses: number;
  delayedJobBindingReads: number;
  sessionCreateRequests: number;
  preferenceUpdateRequests: number;
  remainingPreferenceUpdateFailures: number;
  migrationRequestBodies: string[];
  remainingMigrationFailures: number;
  initialStateReadRequests: number;
}

interface MockJob {
  jobId: string;
  runId: string;
  sessionId: string;
  message: string;
  mode: "completed" | "approval";
  status: "running" | "done" | "cancelled";
  events: Array<{ seq: number; event: Record<string, unknown> }>;
}

interface ProviderProfileMutation {
  label: string;
  provider_type: MockProviderProfile["provider_type"];
  api_base: string;
  api_key_env?: string;
  default_model?: string;
}

interface DelayedSessionVisibility {
  session: MockSession;
  remainingReads: number;
}

export async function installMockProductApi(
  page: Page,
  options: MockProductApiOptions = {},
): Promise<MockProductApiState> {
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
    providerProfiles: structuredClone(options.providerProfiles ?? []),
    memoryTopics: structuredClone(options.memoryTopics ?? {}),
    memoryWorkspaceRequests: [],
    memoryMutationRequests: 0,
    remainingMemoryMutationFailures: options.memoryMutationFailures ?? 0,
    mcpServers: structuredClone(options.mcpServers ?? {}),
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
      ...(options.activeWorkspaceId
        ? { active_workspace_id: options.activeWorkspaceId }
        : {}),
      ...(options.activeSessionId ? { active_session_id: options.activeSessionId } : {}),
    },
    transcriptFailures: { ...(options.transcriptFailures ?? {}) },
    sessionModelConfigUpdateRequests: 0,
    remainingSessionModelConfigFailures:
      options.sessionModelConfigFailures ?? 0,
    disconnectedJobStartResponses: 0,
    delayedJobBindingReads: 0,
    sessionCreateRequests: 0,
    preferenceUpdateRequests: 0,
    remainingPreferenceUpdateFailures: options.preferenceUpdateFailures ?? 0,
    migrationRequestBodies: [],
    remainingMigrationFailures: options.migrationFailures ?? 0,
    initialStateReadRequests: 0,
  };
  const jobs = new Map<string, MockJob>();
  const delayedSessionVisibility = new Map<string, DelayedSessionVisibility>();
  let workspaceCounter = state.workspaces.length;
  let sessionCounter = state.sessions.length;
  let providerProfileCounter = state.providerProfiles.length;
  let migrationReceiptCounter = 0;
  const migratedWorkspaceIds = new Map<string, string>();
  const migratedSessionIds = new Map<string, string>();
  const migratedProfileIds = new Map<string, string>();
  const migrationReceipts = new Map<string, M1BrowserMigrationResponse>();

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
          state.sessionModelConfigs[sessionId] = createMockSessionModelConfig(
            sessionId,
          );
        }
        return [{ source_id: session.source_id, product_session_id: sessionId }];
      });
      const providerProfileMappings = body.provider_profiles.map((profile) => {
        let profileId = migratedProfileIds.get(profile.source_id);
        if (!profileId) {
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
            ...(profile.api_key_env ? { api_key_env: profile.api_key_env } : {}),
            ...(profile.default_model ? { default_model: profile.default_model } : {}),
          });
        }
        return { source_id: profile.source_id, provider_profile_id: profileId };
      });

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
        }
      }
      delete state.mcpServers[workspaceId];
      return route.fulfill({ status: 204, body: "" });
    }
    if (path === "/product/sessions" && method === "GET") {
      const workspaceId = url.searchParams.get("workspace_id");
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
    const transcriptMatch = path.match(
      /^\/product\/sessions\/([^/]+)\/transcript$/u,
    );
    if (transcriptMatch && method === "GET") {
      const sessionId = decodeURIComponent(transcriptMatch[1]!);
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
      return json(
        route,
        state.transcripts[sessionId] ?? emptyTranscript(session),
      );
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
        const created = structuredClone(body);
        state.mcpServers[workspaceId] = [...servers, created].sort((left, right) =>
          left.name.localeCompare(right.name),
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
        const updated: ProductMcpServerConfig = { name, ...body };
        servers[serverIndex] = updated;
        state.mcpServers[workspaceId] = [...servers].sort((left, right) =>
          left.name.localeCompare(right.name),
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
      return json(route, { provider_profiles: state.providerProfiles });
    }
    if (path === "/product/provider-profiles" && method === "POST") {
      const body = request.postDataJSON() as ProviderProfileMutation;
      providerProfileCounter += 1;
      const profile: MockProviderProfile = {
        id: `provider-${providerProfileCounter}`,
        label: body.label,
        provider_type: body.provider_type,
        api_base: body.api_base,
        created_at: NOW,
        updated_at: NOW,
        ...(body.api_key_env ? { api_key_env: body.api_key_env } : {}),
        ...(body.default_model ? { default_model: body.default_model } : {}),
      };
      state.providerProfiles.unshift(profile);
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
      profile.label = body.label;
      profile.provider_type = body.provider_type;
      profile.api_base = body.api_base;
      profile.updated_at = NOW;
      replaceOptional(profile, "api_key_env", body.api_key_env);
      replaceOptional(profile, "default_model", body.default_model);
      return json(route, profile);
    }
    if (providerProfileMatch && method === "DELETE") {
      const profileId = decodeURIComponent(providerProfileMatch[1]!);
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
      state.providerProfiles.splice(profileIndex, 1);
      for (const config of Object.values(state.sessionModelConfigs)) {
        if (config.profile_id === profileId) {
          delete config.profile_id;
          config.revision += 1;
          config.updated_at = NOW;
        }
      }
      return route.fulfill({ status: 204, body: "" });
    }
    return json(route, { code: "product_not_found", error: `unmocked ${method} ${path}` }, 404);
  });

  await page.route("/api/jobs", async (route) => {
    const body = route.request().postDataJSON() as Record<string, unknown> & {
      message: string;
      product_session_id: string;
    };
    state.jobs.push(body);
    const session = state.sessions.find(
      (item) => item.id === body.product_session_id,
    );
    if (!session) {
      return json(route, { code: "product_not_found", error: "session not found" }, 404);
    }
    if (body.resume !== undefined) {
      return json(
        route,
        { code: "product_session_resume_conflict", error: "resume must be omitted" },
        409,
      );
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
    const jobId = `job-${state.jobs.length}`;
    const runId = `run-${state.jobs.length}`;
    const mode = options.mode ?? "completed";
    const output = outputFor(body.message);
    const events =
      mode === "approval"
        ? approvalEvents(jobId, runId, body.message)
        : completedEvents(jobId, runId, body.message, output);
    const job: MockJob = {
      jobId,
      runId,
      sessionId: session.id,
      message: body.message,
      mode,
      status: mode === "approval" ? "running" : "done",
      events,
    };
    jobs.set(jobId, job);
    session.runtime_binding = {
      ordinal,
      runtime_session_id: `runtime-${session.id}`,
      latest_job_id: jobId,
      latest_run_id: runId,
    };
    session.status = mode === "approval" ? "needs_attention" : "idle";
    session.updated_at = NOW;
    const transcript = state.transcripts[session.id] ?? emptyTranscript(session);
    transcript.segments.push(
      transcriptSegment(
        session,
        ordinal,
        jobId,
        runId,
        resumedFromRunId,
        mode === "approval" ? "running" : "done",
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
    if (
      state.disconnectedJobStartResponses <
      (options.disconnectJobStartResponses ?? 0)
    ) {
      state.disconnectedJobStartResponses += 1;
      return route.abort("connectionreset");
    }
    return json(route, started);
  });

  await page.route(/\/api\/jobs\/[^/]+\/events$/u, async (route) => {
    const job = jobFromRoute(route, jobs);
    if (!job) {
      return route.fulfill({ status: 404, body: "" });
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
    return job ? fulfillJobState(route, job) : route.fulfill({ status: 404, body: "" });
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
    segment.events.push(...completed);
    segment.observed_through_seq = 4;
    segment.last_event_seq = 4;
    segment.run_status = "done";
    job.events.push(...completed);
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

export function createMockSession(
  id = "session-1",
  workspaceId = "workspace-1",
  title = "Durable session",
): MockSession {
  return {
    id,
    workspace_id: workspaceId,
    title,
    status: "idle",
    created_at: NOW,
    updated_at: NOW,
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

function emptyTranscript(session: MockSession): MockTranscript {
  return {
    product_session_id: session.id,
    workspace_id: session.workspace_id,
    status: "complete",
    partial_reasons: [],
    segments: [],
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

async function fulfillJobState(
  route: Route,
  job: MockJob,
  events = job.events,
) {
  const approvalPending = job.mode === "approval" && job.status === "running";
  return json(route, {
    job_id: job.jobId,
    run_id: job.runId,
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
