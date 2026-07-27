import { describe, expect, it } from "vitest";

import type { SessionRecord, WorkspaceRecord } from "../state/product-types";
import {
  buildSafeSessionExport,
  createSessionExportDownload,
  downloadSessionExport,
  groupCatalogSessions,
  resolveSessionSelection,
  sortCatalogSessions,
  sortCatalogWorkspaces,
} from "./catalog-settings-model";

const workspaces: WorkspaceRecord[] = [
  {
    id: "ws_recent",
    rootPath: "D:\\private\\recent",
    kind: "folder",
    displayName: "Recent",
    pinned: false,
    lastOpenedAt: "2026-07-27T09:00:00.000Z",
  },
  {
    id: "ws_pinned_old",
    rootPath: "/private/pinned-old",
    kind: "repo",
    displayName: "Pinned old",
    pinned: true,
    lastOpenedAt: "2026-07-25T09:00:00.000Z",
  },
  {
    id: "ws_pinned_new",
    rootPath: "/private/pinned-new",
    kind: "folder",
    displayName: "Pinned new",
    pinned: true,
    lastOpenedAt: "2026-07-26T09:00:00.000Z",
  },
];

const sessions: SessionRecord[] = [
  {
    id: "sess_old",
    workspaceId: "ws_recent",
    title: "Old",
    status: "idle",
    createdAt: "2026-07-25T00:00:00.000Z",
    updatedAt: "2026-07-25T01:00:00.000Z",
    hasDurableTurn: false,
  },
  {
    id: "sess_new",
    workspaceId: "ws_recent",
    title: "New",
    status: "needs_attention",
    createdAt: "2026-07-26T00:00:00.000Z",
    updatedAt: "2026-07-27T01:00:00.000Z",
    activeJobId: "job-secret",
    activeRunId: "run-secret",
    runtimeOrdinal: 3,
    hasDurableTurn: true,
  },
  {
    id: "sess_orphan",
    workspaceId: "ws_missing",
    title: "Orphan",
    status: "error",
    createdAt: "2026-07-24T00:00:00.000Z",
    updatedAt: "2026-07-24T01:00:00.000Z",
    hasDurableTurn: true,
  },
];

describe("catalog settings model", () => {
  it("sorts pinned workspaces first and newest entries within each tier", () => {
    expect(sortCatalogWorkspaces(workspaces).map((item) => item.id)).toEqual([
      "ws_pinned_new",
      "ws_pinned_old",
      "ws_recent",
    ]);
    expect(workspaces.map((item) => item.id)).toEqual([
      "ws_recent",
      "ws_pinned_old",
      "ws_pinned_new",
    ]);
  });

  it("sorts sessions newest first without mutating the source", () => {
    expect(sortCatalogSessions(sessions).map((item) => item.id)).toEqual([
      "sess_new",
      "sess_old",
      "sess_orphan",
    ]);
    expect(sessions[0]?.id).toBe("sess_old");
  });

  it("groups sessions by sorted workspace and preserves orphaned catalog entries", () => {
    const groups = groupCatalogSessions(workspaces, sessions);
    expect(groups.map((group) => group.workspaceId)).toEqual([
      "ws_pinned_new",
      "ws_pinned_old",
      "ws_recent",
      "ws_missing",
    ]);
    expect(groups[2]?.sessions.map((item) => item.id)).toEqual(["sess_new", "sess_old"]);
    expect(groups[3]).toMatchObject({ workspace: null, workspaceId: "ws_missing" });
  });

  it("derives only selections whose session and workspace both exist", () => {
    expect(resolveSessionSelection(workspaces, sessions, "sess_new")).toEqual({
      workspaceId: "ws_recent",
      sessionId: "sess_new",
    });
    expect(resolveSessionSelection(workspaces, sessions, "sess_orphan")).toBeNull();
    expect(resolveSessionSelection(workspaces, sessions, "missing")).toBeNull();
  });

  it("builds a bounded metadata export without local paths or runtime identifiers", () => {
    const session = sessions[1]!;
    const workspace = workspaces[0]!;
    const exported = buildSafeSessionExport(
      session,
      workspace,
      "2026-07-27T10:00:00.000Z",
    );
    expect(exported).toEqual({
      schema_version: 1,
      export_kind: "rove.session.catalog",
      exported_at: "2026-07-27T10:00:00.000Z",
      workspace: {
        id: "ws_recent",
        display_name: "Recent",
        kind: "folder",
      },
      session: {
        id: "sess_new",
        title: "New",
        status: "needs_attention",
        created_at: "2026-07-26T00:00:00.000Z",
        updated_at: "2026-07-27T01:00:00.000Z",
        has_durable_turn: true,
        runtime_ordinal: 3,
      },
    });
    const serialized = JSON.stringify(exported);
    expect(serialized).not.toContain("private");
    expect(serialized).not.toContain("job-secret");
    expect(serialized).not.toContain("run-secret");
    expect(serialized).not.toContain("provider");
  });

  it("uses the session workspace id when supplied workspace metadata does not match", () => {
    const exported = buildSafeSessionExport(
      sessions[1]!,
      workspaces[1]!,
      "2026-07-27T10:00:00.000Z",
    );
    expect(exported.workspace).toEqual({
      id: "ws_recent",
      display_name: null,
      kind: null,
    });
  });

  it("creates an ASCII JSON download with a sanitized bounded filename", () => {
    const download = createSessionExportDownload(
      { ...sessions[1]!, id: "session/../../秘密 with spaces" },
      workspaces[0]!,
      "2026-07-27T10:00:00.000Z",
    );
    expect(download.filename).toBe("rove-session-session-with-spaces.json");
    expect(download.mediaType).toBe("application/json");
    expect(download.content.endsWith("\n")).toBe(true);
    expect(JSON.parse(download.content)).toMatchObject({
      schema_version: 1,
      session: { id: "session/../../秘密 with spaces" },
    });
    expect(() => downloadSessionExport(download)).toThrow(/browser environment/i);
  });
});
