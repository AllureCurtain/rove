import type { CreateJobWorkspaceKind, ProviderProfile, ProviderType } from "../lib/rove-types";

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
  /**
   * True once this product session has completed at least one durable turn
   * under its workspace root. Subsequent turns must hard-resume (`resume: "latest"`).
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

export interface ActiveProviderSelection {
  mode: "default" | "profile";
  profileId?: string;
  model: string;
  approval: "ask" | "auto" | "never";
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

export function newId(prefix: string): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return `${prefix}_${crypto.randomUUID()}`;
  }
  return `${prefix}_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 8)}`;
}
