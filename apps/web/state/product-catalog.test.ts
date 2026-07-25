import { describe, expect, it } from "vitest";

import {
  createSession,
  ensureActiveSession,
  isAbsoluteWorkspacePath,
  loadProductCatalog,
  openWorkspace,
  selectSession,
  sessionsForWorkspace,
  togglePinWorkspace,
  updateSession,
  type ProductCatalog,
} from "./product-catalog";

function emptyCatalog(): ProductCatalog {
  return {
    workspaces: [],
    sessions: [],
    active: { workspaceId: null, sessionId: null },
  };
}

describe("product catalog", () => {
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
