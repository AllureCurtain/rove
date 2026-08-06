import type { CreateJobWorkspaceKind, ProviderProfile, ProviderType } from "../lib/rove-types";
import type {
  ProductProviderProfile,
  ProductSession,
  ProductSessionModelConfig,
  ProductWorkspace,
} from "../product/product-api-types";
import type { ProductReasoningPreference } from "../product/product-api-types";

export type WorkspaceKind = CreateJobWorkspaceKind;

export interface WorkspaceRecord {
  id: string;
  rootPath: string;
  kind: WorkspaceKind;
  displayName: string;
  pinned: boolean;
  lastOpenedAt: string;
}

export type SessionStatus = "idle" | "running" | "error" | "needs_attention";

export interface SessionRecord {
  id: string;
  workspaceId: string;
  title: string;
  createdAt: string;
  updatedAt: string;
  status: SessionStatus;
  /** Last successful/known job for this product session (UI bookkeeping). */
  activeJobId?: string | null;
  activeRunId?: string | null;
  resumedFromRunId?: string | null;
  runtimeOrdinal?: number | null;
  /** Immutable parent relation for a forked product session. */
  parentSessionId?: string | null;
  /** Exact final source boundary retained by a forked session. */
  forkPointRunId?: string | null;
  forkPointSeq?: number | null;
  /**
   * True once this product session has completed at least one durable turn
   * under its workspace root. The server uses this binding for exact resume.
   */
  hasDurableTurn: boolean;
}

export interface ProviderProfileRecord {
  id: string;
  label: string;
  providerType: ProviderType;
  apiBase: string;
  apiKeyEnv?: string;
  defaultModel?: string;
  updatedAt: string;
}

export type ProviderProfileInput = Omit<ProviderProfileRecord, "id" | "updatedAt">;

export interface ActiveProviderSelection {
  mode: "default" | "profile";
  profileId?: string;
  model: string;
  approval: "ask" | "auto" | "never";
  maxSteps: number;
}

export interface SessionModelConfig {
  sessionId: string;
  profileId?: string;
  model: string;
  reasoning: ProductReasoningPreference;
  maxSteps: number;
  revision: number;
  updatedAt: string;
}

export interface SessionModelConfigInput {
  profileId?: string;
  model: string;
  reasoning: ProductReasoningPreference;
  maxSteps: number;
}

export function workspaceDisplayName(rootPath: string): string {
  const normalized = rootPath.replace(/[\\/]+$/, "");
  const parts = normalized.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] || normalized || "Workspace";
}

export function toApiProviderProfile(
  record: ProviderProfileRecord,
): ProviderProfile {
  return {
    provider_type: record.providerType,
    name: record.label || undefined,
    api_base: record.apiBase,
    api_key_env: record.apiKeyEnv,
  };
}

export function fromProductWorkspace(workspace: ProductWorkspace): WorkspaceRecord {
  return {
    id: workspace.id,
    rootPath: workspace.canonical_root,
    kind: workspace.kind,
    displayName: workspace.display_name,
    pinned: workspace.pinned,
    lastOpenedAt: workspace.last_opened_at,
  };
}

export function fromProductSession(session: ProductSession): SessionRecord {
  return {
    id: session.id,
    workspaceId: session.workspace_id,
    title: session.title,
    createdAt: session.created_at,
    updatedAt: session.updated_at,
    status: session.status === "archived" ? "idle" : session.status,
    activeJobId: session.runtime_binding?.latest_job_id ?? null,
    activeRunId: session.runtime_binding?.latest_run_id ?? null,
    resumedFromRunId: null,
    runtimeOrdinal: session.runtime_binding?.ordinal ?? null,
    parentSessionId: session.parent_session_id ?? null,
    forkPointRunId: session.fork_point_run_id ?? null,
    forkPointSeq: session.fork_point_seq ?? null,
    hasDurableTurn: session.runtime_binding !== undefined,
  };
}

export function fromProductProviderProfile(
  profile: ProductProviderProfile,
): ProviderProfileRecord {
  return {
    id: profile.id,
    label: profile.label,
    providerType: profile.provider_type,
    apiBase: profile.api_base,
    apiKeyEnv: profile.api_key_env,
    defaultModel: profile.default_model,
    updatedAt: profile.updated_at,
  };
}

export function fromProductSessionModelConfig(
  config: ProductSessionModelConfig,
): SessionModelConfig {
  return {
    sessionId: config.product_session_id,
    profileId: config.profile_id,
    model: config.model,
    reasoning: config.reasoning,
    maxSteps: config.max_steps,
    revision: config.revision,
    updatedAt: config.updated_at,
  };
}

export function newId(prefix: string): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return `${prefix}_${crypto.randomUUID()}`;
  }
  return `${prefix}_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 8)}`;
}
