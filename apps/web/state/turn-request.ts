import type {
  CreateJobRequest,
  CreateJobWorkspace,
} from "../lib/rove-types";
import type {
  ActiveProviderSelection,
  ProviderProfileRecord,
  SessionRecord,
  WorkspaceRecord,
} from "./product-types";

export interface BuildTurnRequestInput {
  message: string;
  workspace: WorkspaceRecord;
  session: SessionRecord;
}

export class ProviderSelectionError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ProviderSelectionError";
  }
}

/**
 * Fail closed when the stored provider selection cannot be satisfied.
 *
 * The server owns provider, approval, and step limits for a run, so a turn
 * request carries none of them. This check is only a local consistency check on
 * catalog data the browser already holds: it refuses to submit a turn that names
 * a profile the catalog no longer contains, so the user is not shown an
 * optimistic turn that is guaranteed to fail.
 */
export function assertProviderSelectionIsSatisfiable(
  selection: ActiveProviderSelection | null | undefined,
  profiles: ProviderProfileRecord[],
): void {
  if (!selection || selection.mode !== "profile") {
    return;
  }
  if (!selection.profileId) {
    throw new ProviderSelectionError(
      "The selected provider profile is missing. Choose a provider in Settings.",
    );
  }
  if (!profiles.some((profile) => profile.id === selection.profileId)) {
    throw new ProviderSelectionError(
      "The selected provider profile is no longer available. Choose another provider in Settings.",
    );
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
  return {
    message: input.message,
    workspace,
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
