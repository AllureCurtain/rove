import type {
  CreateJobRequest,
  CreateJobWorkspace,
  ProviderProfile,
} from "../lib/rove-types";
import type {
  ActiveProviderSelection,
  ProviderProfileRecord,
  SessionRecord,
  WorkspaceRecord,
} from "./product-types";
import { toApiProviderProfile } from "./product-types";

export interface BuildTurnRequestInput {
  message: string;
  workspace: WorkspaceRecord;
  session: SessionRecord;
  selection: ActiveProviderSelection;
  profiles: ProviderProfileRecord[];
}

export class ProviderSelectionError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ProviderSelectionError";
  }
}

/**
 * Build a create-job payload for a product chat turn.
 *
 * Product continuity is server-owned. Every product turn carries the exact
 * product session id and must omit the lower-level client resume field.
 */
export function buildTurnJobRequest(input: BuildTurnRequestInput): CreateJobRequest {
  const workspace = toCreateJobWorkspace(input.workspace);
  const provider = resolveProvider(input.selection, input.profiles);
  return {
    message: input.message,
    model: input.selection.model.trim() || undefined,
    max_steps: input.selection.maxSteps || undefined,
    approval: input.selection.approval,
    workspace,
    provider,
    product_session_id: input.session.id,
  };
}

export function toCreateJobWorkspace(workspace: WorkspaceRecord): CreateJobWorkspace {
  if (workspace.kind === "task") {
    return {
      kind: "task",
      name: workspace.displayName,
      base: workspace.rootPath,
    };
  }
  return {
    kind: workspace.kind,
    root: workspace.rootPath,
  };
}

function resolveProvider(
  selection: ActiveProviderSelection,
  profiles: ProviderProfileRecord[],
): ProviderProfile | undefined {
  if (selection.mode !== "profile" || !selection.profileId) {
    if (selection.mode === "profile") {
      throw new ProviderSelectionError(
        "The selected provider profile is missing. Choose a provider in Settings.",
      );
    }
    return undefined;
  }
  const profile = profiles.find((item) => item.id === selection.profileId);
  if (!profile) {
    throw new ProviderSelectionError(
      "The selected provider profile is no longer available. Choose another provider in Settings.",
    );
  }
  return toApiProviderProfile(profile);
}

export function isHardResumeError(message: string): boolean {
  const lower = message.toLowerCase();
  return (
    lower.includes("resume") ||
    lower.includes("product_session") ||
    lower.includes("product session") ||
    lower.includes("runtime state") ||
    lower.includes("binding") ||
    lower.includes("task_state") ||
    lower.includes("no durable") ||
    lower.includes("not resumable") ||
    lower.includes("fail-closed") ||
    lower.includes("fail closed")
  );
}
