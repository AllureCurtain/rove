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

/**
 * Build a create-job payload for a product chat turn.
 *
 * Continuity rule (hard resume only):
 * - First durable turn in a product session: no resume field.
 * - Subsequent turns: always `resume: "latest"` under the same workspace root.
 * Soft stitch (new job without resume + frontend-only transcript) is forbidden.
 */
export function buildTurnJobRequest(input: BuildTurnRequestInput): CreateJobRequest {
  const workspace = toCreateJobWorkspace(input.workspace);
  const provider = resolveProvider(input.selection, input.profiles);
  const resume = input.session.hasDurableTurn ? ("latest" as const) : undefined;

  return {
    message: input.message,
    model: input.selection.model.trim() || undefined,
    max_steps: input.selection.maxSteps || undefined,
    approval: input.selection.approval,
    resume,
    workspace,
    provider,
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
    return undefined;
  }
  const profile = profiles.find((item) => item.id === selection.profileId);
  return profile ? toApiProviderProfile(profile) : undefined;
}

export function isHardResumeError(message: string): boolean {
  const lower = message.toLowerCase();
  return (
    lower.includes("resume") ||
    lower.includes("task_state") ||
    lower.includes("no durable") ||
    lower.includes("not resumable") ||
    lower.includes("fail-closed") ||
    lower.includes("fail closed")
  );
}
