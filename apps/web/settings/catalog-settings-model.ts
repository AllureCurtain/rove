import type { SessionRecord, WorkspaceRecord } from "../state/product-types";

export const SESSION_EXPORT_SCHEMA_VERSION = 1 as const;
export const SESSION_EXPORT_KIND = "rove.session.catalog" as const;

export interface CatalogSessionSelection {
  workspaceId: string;
  sessionId: string;
}

export interface CatalogSessionGroup {
  workspaceId: string;
  workspace: WorkspaceRecord | null;
  sessions: SessionRecord[];
}

/**
 * A deliberately narrow catalog export. Local roots, provider configuration,
 * transcript content, and runtime identifiers are excluded.
 */
export interface SafeSessionExport {
  schema_version: typeof SESSION_EXPORT_SCHEMA_VERSION;
  export_kind: typeof SESSION_EXPORT_KIND;
  exported_at: string;
  workspace: {
    id: string;
    display_name: string | null;
    kind: WorkspaceRecord["kind"] | null;
  };
  session: {
    id: string;
    title: string;
    status: SessionRecord["status"];
    created_at: string;
    updated_at: string;
    has_durable_turn: boolean;
    runtime_ordinal: number | null;
  };
}

export interface SessionExportDownload {
  filename: string;
  mediaType: "application/json";
  content: string;
}

function compareText(left: string, right: string): number {
  return left.localeCompare(right, undefined, { sensitivity: "base" });
}

export function sortCatalogWorkspaces(
  workspaces: readonly WorkspaceRecord[],
): WorkspaceRecord[] {
  return [...workspaces].sort((left, right) => {
    if (left.pinned !== right.pinned) {
      return left.pinned ? -1 : 1;
    }
    const byLastOpened = right.lastOpenedAt.localeCompare(left.lastOpenedAt);
    if (byLastOpened !== 0) {
      return byLastOpened;
    }
    const byName = compareText(left.displayName, right.displayName);
    return byName !== 0 ? byName : left.id.localeCompare(right.id);
  });
}

export function sortCatalogSessions(
  sessions: readonly SessionRecord[],
): SessionRecord[] {
  return [...sessions].sort((left, right) => {
    const byUpdatedAt = right.updatedAt.localeCompare(left.updatedAt);
    if (byUpdatedAt !== 0) {
      return byUpdatedAt;
    }
    const byTitle = compareText(left.title, right.title);
    return byTitle !== 0 ? byTitle : left.id.localeCompare(right.id);
  });
}

export function groupCatalogSessions(
  workspaces: readonly WorkspaceRecord[],
  sessions: readonly SessionRecord[],
): CatalogSessionGroup[] {
  const groups = new Map<string, CatalogSessionGroup>();
  const sortedWorkspaces = sortCatalogWorkspaces(workspaces);
  const workspaceOrder = new Map(
    sortedWorkspaces.map((workspace, index) => [workspace.id, index]),
  );

  for (const workspace of sortedWorkspaces) {
    groups.set(workspace.id, {
      workspaceId: workspace.id,
      workspace,
      sessions: [],
    });
  }

  for (const session of sessions) {
    const existing = groups.get(session.workspaceId);
    if (existing) {
      existing.sessions.push(session);
      continue;
    }
    groups.set(session.workspaceId, {
      workspaceId: session.workspaceId,
      workspace: null,
      sessions: [session],
    });
  }

  return [...groups.values()]
    .map((group) => ({
      ...group,
      sessions: sortCatalogSessions(group.sessions),
    }))
    .sort((left, right) => {
      const leftOrder = workspaceOrder.get(left.workspaceId);
      const rightOrder = workspaceOrder.get(right.workspaceId);
      if (leftOrder !== undefined && rightOrder !== undefined) {
        return leftOrder - rightOrder;
      }
      if (leftOrder !== undefined) return -1;
      if (rightOrder !== undefined) return 1;
      return left.workspaceId.localeCompare(right.workspaceId);
    });
}

export function resolveSessionSelection(
  workspaces: readonly WorkspaceRecord[],
  sessions: readonly SessionRecord[],
  sessionId: string,
): CatalogSessionSelection | null {
  const session = sessions.find((item) => item.id === sessionId);
  if (!session || !workspaces.some((item) => item.id === session.workspaceId)) {
    return null;
  }
  return { workspaceId: session.workspaceId, sessionId: session.id };
}

export function buildSafeSessionExport(
  session: SessionRecord,
  workspace: WorkspaceRecord | null,
  exportedAt = new Date().toISOString(),
): SafeSessionExport {
  const matchingWorkspace = workspace?.id === session.workspaceId ? workspace : null;
  return {
    schema_version: SESSION_EXPORT_SCHEMA_VERSION,
    export_kind: SESSION_EXPORT_KIND,
    exported_at: exportedAt,
    workspace: {
      id: session.workspaceId,
      display_name: matchingWorkspace?.displayName ?? null,
      kind: matchingWorkspace?.kind ?? null,
    },
    session: {
      id: session.id,
      title: session.title,
      status: session.status,
      created_at: session.createdAt,
      updated_at: session.updatedAt,
      has_durable_turn: session.hasDurableTurn,
      runtime_ordinal: session.runtimeOrdinal ?? null,
    },
  };
}

function safeFilenamePart(value: string): string {
  return value.replace(/[^a-zA-Z0-9_-]/g, "-").replace(/-+/g, "-").slice(0, 64);
}

export function createSessionExportDownload(
  session: SessionRecord,
  workspace: WorkspaceRecord | null,
  exportedAt = new Date().toISOString(),
): SessionExportDownload {
  const payload = buildSafeSessionExport(session, workspace, exportedAt);
  const safeSessionId = safeFilenamePart(session.id) || "session";
  return {
    filename: `rove-session-${safeSessionId}.json`,
    mediaType: "application/json",
    content: `${JSON.stringify(payload, null, 2)}\n`,
  };
}

export function downloadSessionExport(download: SessionExportDownload): void {
  if (typeof document === "undefined" || typeof URL === "undefined") {
    throw new Error("Session export downloads require a browser environment.");
  }

  const blob = new Blob([download.content], { type: download.mediaType });
  const objectUrl = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = objectUrl;
  anchor.download = download.filename;
  anchor.rel = "noopener";
  anchor.hidden = true;
  document.body.append(anchor);
  try {
    anchor.click();
  } finally {
    anchor.remove();
    URL.revokeObjectURL(objectUrl);
  }
}
