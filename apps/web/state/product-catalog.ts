import type { PlatformAdapter } from "../platform/types";
import { webPlatform } from "../platform/web";
import { readJsonStore, writeJsonStore } from "./local-store";
import {
  newId,
  workspaceDisplayName,
  type SessionRecord,
  type SessionStatus,
  type WorkspaceKind,
  type WorkspaceRecord,
} from "./product-types";

const WORKSPACES_KEY = "rove.product.workspaces";
const SESSIONS_KEY = "rove.product.sessions";
const ACTIVE_KEY = "rove.product.active";

export interface ActiveSelection {
  workspaceId: string | null;
  sessionId: string | null;
}

export interface ProductCatalog {
  workspaces: WorkspaceRecord[];
  sessions: SessionRecord[];
  active: ActiveSelection;
}

function emptyCatalog(): ProductCatalog {
  return {
    workspaces: [],
    sessions: [],
    active: { workspaceId: null, sessionId: null },
  };
}

export function loadProductCatalog(
  platform: PlatformAdapter = webPlatform,
): ProductCatalog {
  const workspaces = readJsonStore<WorkspaceRecord[]>(WORKSPACES_KEY, [], platform);
  const sessions = readJsonStore<SessionRecord[]>(SESSIONS_KEY, [], platform);
  const active = readJsonStore<ActiveSelection>(
    ACTIVE_KEY,
    { workspaceId: null, sessionId: null },
    platform,
  );
  return { workspaces, sessions, active };
}

export function saveProductCatalog(
  catalog: ProductCatalog,
  platform: PlatformAdapter = webPlatform,
): void {
  writeJsonStore(WORKSPACES_KEY, catalog.workspaces, platform);
  writeJsonStore(SESSIONS_KEY, catalog.sessions, platform);
  writeJsonStore(ACTIVE_KEY, catalog.active, platform);
}

export function normalizeWorkspacePath(path: string): string {
  return path.trim().replace(/[\\/]+$/, "");
}

export function isAbsoluteWorkspacePath(path: string): boolean {
  const value = path.trim();
  if (!value) {
    return false;
  }
  // Windows drive path or UNC, or POSIX absolute.
  return /^([a-zA-Z]:[\\/]|\\\\|\/)/.test(value);
}

export function openWorkspace(
  catalog: ProductCatalog,
  rootPath: string,
  kind: WorkspaceKind = "folder",
): ProductCatalog {
  const normalized = normalizeWorkspacePath(rootPath);
  if (!normalized || !isAbsoluteWorkspacePath(normalized)) {
    throw new Error("Workspace path must be an absolute local directory.");
  }

  const existing = catalog.workspaces.find(
    (workspace) =>
      normalizeWorkspacePath(workspace.rootPath).toLowerCase() ===
      normalized.toLowerCase(),
  );

  const now = new Date().toISOString();
  let workspaces: WorkspaceRecord[];
  let workspaceId: string;

  if (existing) {
    workspaceId = existing.id;
    workspaces = catalog.workspaces.map((workspace) =>
      workspace.id === existing.id
        ? {
            ...workspace,
            kind,
            rootPath: normalized,
            displayName: workspaceDisplayName(normalized),
            lastOpenedAt: now,
          }
        : workspace,
    );
  } else {
    workspaceId = newId("ws");
    const record: WorkspaceRecord = {
      id: workspaceId,
      rootPath: normalized,
      kind,
      displayName: workspaceDisplayName(normalized),
      pinned: false,
      lastOpenedAt: now,
    };
    workspaces = [record, ...catalog.workspaces];
  }

  return {
    ...catalog,
    workspaces,
    active: {
      workspaceId,
      sessionId: catalog.active.sessionId,
    },
  };
}

export function togglePinWorkspace(
  catalog: ProductCatalog,
  workspaceId: string,
): ProductCatalog {
  return {
    ...catalog,
    workspaces: catalog.workspaces.map((workspace) =>
      workspace.id === workspaceId
        ? { ...workspace, pinned: !workspace.pinned }
        : workspace,
    ),
  };
}

export function removeWorkspace(
  catalog: ProductCatalog,
  workspaceId: string,
): ProductCatalog {
  const workspaces = catalog.workspaces.filter((w) => w.id !== workspaceId);
  const sessions = catalog.sessions.filter((s) => s.workspaceId !== workspaceId);
  const active =
    catalog.active.workspaceId === workspaceId
      ? { workspaceId: null, sessionId: null }
      : catalog.active.sessionId &&
          !sessions.some((s) => s.id === catalog.active.sessionId)
        ? { ...catalog.active, sessionId: null }
        : catalog.active;
  return { workspaces, sessions, active };
}

export function createSession(
  catalog: ProductCatalog,
  workspaceId: string,
  title = "New session",
): ProductCatalog {
  const now = new Date().toISOString();
  const session: SessionRecord = {
    id: newId("sess"),
    workspaceId,
    title,
    createdAt: now,
    updatedAt: now,
    status: "idle",
    activeJobId: null,
    activeRunId: null,
    resumedFromRunId: null,
    hasDurableTurn: false,
  };
  return {
    ...catalog,
    sessions: [session, ...catalog.sessions],
    active: {
      workspaceId,
      sessionId: session.id,
    },
  };
}

export function selectSession(
  catalog: ProductCatalog,
  workspaceId: string,
  sessionId: string,
): ProductCatalog {
  return {
    ...catalog,
    active: { workspaceId, sessionId },
  };
}

export function selectWorkspace(
  catalog: ProductCatalog,
  workspaceId: string,
): ProductCatalog {
  const sessions = catalog.sessions
    .filter((session) => session.workspaceId === workspaceId)
    .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
  return {
    ...catalog,
    active: {
      workspaceId,
      sessionId: sessions[0]?.id ?? null,
    },
  };
}

export function updateSession(
  catalog: ProductCatalog,
  sessionId: string,
  patch: Partial<
    Pick<
      SessionRecord,
      | "title"
      | "status"
      | "activeJobId"
      | "activeRunId"
      | "resumedFromRunId"
      | "hasDurableTurn"
      | "updatedAt"
    >
  >,
): ProductCatalog {
  const now = new Date().toISOString();
  return {
    ...catalog,
    sessions: catalog.sessions.map((session) =>
      session.id === sessionId
        ? {
            ...session,
            ...patch,
            updatedAt: patch.updatedAt ?? now,
          }
        : session,
    ),
  };
}

export function setSessionStatus(
  catalog: ProductCatalog,
  sessionId: string,
  status: SessionStatus,
): ProductCatalog {
  return updateSession(catalog, sessionId, { status });
}

export function sessionsForWorkspace(
  catalog: ProductCatalog,
  workspaceId: string,
): SessionRecord[] {
  return catalog.sessions
    .filter((session) => session.workspaceId === workspaceId)
    .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
}

export function findWorkspace(
  catalog: ProductCatalog,
  workspaceId: string | null,
): WorkspaceRecord | null {
  if (!workspaceId) {
    return null;
  }
  return catalog.workspaces.find((w) => w.id === workspaceId) ?? null;
}

export function findSession(
  catalog: ProductCatalog,
  sessionId: string | null,
): SessionRecord | null {
  if (!sessionId) {
    return null;
  }
  return catalog.sessions.find((s) => s.id === sessionId) ?? null;
}

export function sortedWorkspaces(catalog: ProductCatalog): WorkspaceRecord[] {
  return [...catalog.workspaces].sort((a, b) => {
    if (a.pinned !== b.pinned) {
      return a.pinned ? -1 : 1;
    }
    return b.lastOpenedAt.localeCompare(a.lastOpenedAt);
  });
}

export function ensureActiveSession(
  catalog: ProductCatalog,
  workspaceId: string,
): ProductCatalog {
  const existing = sessionsForWorkspace(catalog, workspaceId);
  if (existing.length > 0) {
    return selectSession(catalog, workspaceId, existing[0].id);
  }
  return createSession(catalog, workspaceId);
}
