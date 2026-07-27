import { describe, expect, it } from "vitest";

import {
  createSession,
  ensureActiveSession,
  isAbsoluteWorkspacePath,
  loadProductCatalog,
  openWorkspace,
  productCatalogFromApi,
  replaceServerSessions,
  selectSession,
  sessionsForWorkspace,
  togglePinWorkspace,
  updateSession,
  type ProductCatalog,
} from "./product-catalog";
import type {
  ProductPreferences,
  ProductSession,
  ProductWorkspace,
} from "../product/product-api-types";

function emptyCatalog(): ProductCatalog {
  return {
    workspaces: [],
    sessions: [],
    active: { workspaceId: null, sessionId: null },
  };
}

describe("product catalog", () => {
  it("maps the server catalog and preferences as the active authority", () => {
    const workspaces: ProductWorkspace[] = [
      {
        id: "ws_server",
        canonical_root: "D:/tmp/server",
        kind: "folder",
        display_name: "server",
        pinned: true,
        last_opened_at: "2026-07-26T01:00:00.000Z",
        created_at: "2026-07-26T00:00:00.000Z",
        updated_at: "2026-07-26T01:00:00.000Z",
      },
    ];
    const sessions: ProductSession[] = [
      {
        id: "sess_server",
        workspace_id: "ws_server",
        title: "Durable session",
        status: "running",
        runtime_binding: {
          ordinal: 2,
          runtime_session_id: "runtime-session",
          latest_job_id: "job-2",
          latest_run_id: "run-2",
        },
        created_at: "2026-07-26T00:00:00.000Z",
        updated_at: "2026-07-26T01:00:00.000Z",
      },
      {
        id: "sess_archived",
        workspace_id: "ws_server",
        title: "Archived",
        status: "archived",
        created_at: "2026-07-25T00:00:00.000Z",
        updated_at: "2026-07-25T01:00:00.000Z",
      },
    ];
    const preferences: ProductPreferences = {
      schema_version: 1,
      theme: "dark",
      active_workspace_id: "ws_server",
      active_session_id: "sess_server",
    };

    const catalog = productCatalogFromApi(workspaces, sessions, preferences);

    expect(catalog.active).toEqual({
      workspaceId: "ws_server",
      sessionId: "sess_server",
    });
    expect(catalog.sessions).toHaveLength(1);
    expect(catalog.sessions[0]).toMatchObject({
      status: "running",
      activeJobId: "job-2",
      activeRunId: "run-2",
      runtimeOrdinal: 2,
      hasDurableTurn: true,
    });
  });

  it("refreshes durable session statuses without changing valid focus", () => {
    const catalog: ProductCatalog = {
      workspaces: [
        {
          id: "ws_server",
          rootPath: "/tmp/server",
          kind: "folder",
          displayName: "server",
          pinned: false,
          lastOpenedAt: "2026-07-26T00:00:00.000Z",
        },
      ],
      sessions: [],
      active: { workspaceId: "ws_server", sessionId: "sess_server" },
    };
    const refreshed = replaceServerSessions(catalog, ["ws_server"], [
      {
        id: "sess_server",
        workspace_id: "ws_server",
        title: "Observed",
        status: "needs_attention",
        runtime_binding: {
          ordinal: 1,
          runtime_session_id: "runtime-session",
          latest_job_id: "job-1",
          latest_run_id: "run-1",
        },
        created_at: "2026-07-26T00:00:00.000Z",
        updated_at: "2026-07-26T01:00:00.000Z",
      },
    ]);

    expect(refreshed.active.sessionId).toBe("sess_server");
    expect(refreshed.sessions[0]?.status).toBe("needs_attention");
  });

  it("treats an empty workspace session list as authoritative", () => {
    const catalog: ProductCatalog = {
      workspaces: [
        {
          id: "ws_server",
          rootPath: "/tmp/server",
          kind: "folder",
          displayName: "server",
          pinned: false,
          lastOpenedAt: "2026-07-26T00:00:00.000Z",
        },
      ],
      sessions: [
        {
          id: "stale-session",
          workspaceId: "ws_server",
          title: "Stale",
          status: "running",
          createdAt: "2026-07-26T00:00:00.000Z",
          updatedAt: "2026-07-26T00:00:00.000Z",
          hasDurableTurn: true,
        },
      ],
      active: { workspaceId: "ws_server", sessionId: "stale-session" },
    };

    const refreshed = replaceServerSessions(catalog, ["ws_server"], []);

    expect(refreshed.sessions).toEqual([]);
    expect(refreshed.active).toEqual({
      workspaceId: "ws_server",
      sessionId: null,
    });
  });

  it("rejects non-absolute workspace paths", () => {
    expect(isAbsoluteWorkspacePath("relative/path")).toBe(false);
    expect(isAbsoluteWorkspacePath("D:\\\\Study\\\\rove")).toBe(true);
    expect(isAbsoluteWorkspacePath("/home/user/project")).toBe(true);
    expect(() => openWorkspace(emptyCatalog(), "relative/path")).toThrow(
      /absolute/i,
    );
  });

  it("opens a folder workspace and can create sessions under it", () => {
    const opened = openWorkspace(
      emptyCatalog(),
      "D:\\\\Study\\\\project\\\\agent\\\\rove",
      "folder",
    );
    expect(opened.workspaces).toHaveLength(1);
    expect(opened.workspaces[0]?.kind).toBe("folder");
    expect(opened.workspaces[0]?.displayName).toBe("rove");
    expect(opened.active.workspaceId).toBe(opened.workspaces[0]?.id);

    const withSession = createSession(opened, opened.workspaces[0]!.id, "Explore");
    expect(withSession.sessions).toHaveLength(1);
    expect(withSession.active.sessionId).toBe(withSession.sessions[0]?.id);
    expect(withSession.sessions[0]?.hasDurableTurn).toBe(false);
  });

  it("reopening the same path updates lastOpenedAt and keeps id", () => {
    const first = openWorkspace(emptyCatalog(), "/tmp/demo", "repo");
    const id = first.workspaces[0]!.id;
    const second = openWorkspace(first, "/tmp/demo/", "repo");
    expect(second.workspaces).toHaveLength(1);
    expect(second.workspaces[0]?.id).toBe(id);
    expect(second.workspaces[0]?.rootPath).toBe("/tmp/demo");
  });

  it("pins workspaces ahead of recents ordering via sorted consumers", () => {
    let catalog = openWorkspace(emptyCatalog(), "/tmp/a", "folder");
    catalog = openWorkspace(catalog, "/tmp/b", "folder");
    const bId = catalog.workspaces.find((w) => w.rootPath === "/tmp/b")!.id;
    catalog = togglePinWorkspace(catalog, bId);
    const pinned = catalog.workspaces.find((w) => w.id === bId);
    expect(pinned?.pinned).toBe(true);
  });

  it("ensureActiveSession reuses newest session or creates one", () => {
    const opened = openWorkspace(emptyCatalog(), "/tmp/ws", "folder");
    const wsId = opened.workspaces[0]!.id;
    const ensured = ensureActiveSession(opened, wsId);
    expect(sessionsForWorkspace(ensured, wsId)).toHaveLength(1);

    const again = ensureActiveSession(ensured, wsId);
    expect(sessionsForWorkspace(again, wsId)).toHaveLength(1);
    expect(again.active.sessionId).toBe(ensured.active.sessionId);
  });

  it("tracks durable turn flag for hard resume bookkeeping", () => {
    const opened = openWorkspace(emptyCatalog(), "/tmp/ws", "folder");
    const withSession = createSession(opened, opened.workspaces[0]!.id);
    const sessionId = withSession.sessions[0]!.id;
    const updated = updateSession(withSession, sessionId, {
      hasDurableTurn: true,
      activeJobId: "job-1",
      activeRunId: "run-1",
    });
    expect(updated.sessions[0]?.hasDurableTurn).toBe(true);
    const selected = selectSession(
      updated,
      opened.workspaces[0]!.id,
      sessionId,
    );
    expect(selected.active.sessionId).toBe(sessionId);
  });

  it("loads an empty catalog when storage is empty", () => {
    const platform = {
      host: "web" as const,
      pickWorkspacePath: async () => null,
      getThemePreference: () => "light" as const,
      setThemePreference: () => undefined,
      resolveTheme: () => "light" as const,
      storageGet: () => null,
      storageSet: () => undefined,
      storageRemove: () => undefined,
    };
    expect(loadProductCatalog(platform)).toEqual(emptyCatalog());
  });
});
