import { describe, expect, it } from "vitest";

import type { ProductPreferences } from "../product/product-api-types";
import type { ProductCatalog } from "./product-catalog";
import {
  defaultWorkspaceHref,
  parseProductRoute,
  preferredProductHref,
  sessionHref,
  settingsHref,
} from "./product-route";

const catalog: ProductCatalog = {
  workspaces: [
    {
      id: "ws one",
      rootPath: "/tmp/one",
      kind: "folder",
      displayName: "one",
      pinned: false,
      lastOpenedAt: "2026-07-26T00:00:00.000Z",
    },
    {
      id: "ws-two",
      rootPath: "/tmp/two",
      kind: "repo",
      displayName: "two",
      pinned: false,
      lastOpenedAt: "2026-07-25T00:00:00.000Z",
    },
  ],
  sessions: [
    {
      id: "session/one",
      workspaceId: "ws one",
      title: "One",
      createdAt: "2026-07-26T00:00:00.000Z",
      updatedAt: "2026-07-26T00:00:00.000Z",
      status: "idle",
      hasDurableTurn: true,
    },
    {
      id: "session-two",
      workspaceId: "ws-two",
      title: "Two",
      createdAt: "2026-07-25T00:00:00.000Z",
      updatedAt: "2026-07-25T00:00:00.000Z",
      status: "idle",
      hasDurableTurn: false,
    },
  ],
  active: { workspaceId: null, sessionId: null },
};

const preferences: ProductPreferences = {
  schema_version: 1,
  theme: "light",
  active_workspace_id: "ws-two",
  active_session_id: "session-two",
};

describe("product routes", () => {
  it("parses every durable product route and rejects unknown sections", () => {
    expect(parseProductRoute("/")).toEqual({ kind: "root" });
    expect(parseProductRoute("/w/ws%20one")).toEqual({
      kind: "workspace",
      workspaceId: "ws one",
    });
    expect(parseProductRoute("/w/ws%20one/s/session%2Fone")).toEqual({
      kind: "session",
      workspaceId: "ws one",
      sessionId: "session/one",
    });
    expect(parseProductRoute("/settings")).toEqual({
      kind: "settings",
      section: null,
    });
    expect(parseProductRoute("/settings/memory")).toEqual({
      kind: "settings",
      section: "memory",
    });
    expect(parseProductRoute("/settings/not-real")).toEqual({ kind: "invalid" });
  });

  it("redirects root and workspace routes using server preferences", () => {
    expect(preferredProductHref(catalog, preferences)).toBe(
      "/w/ws-two/s/session-two",
    );
    expect(defaultWorkspaceHref(catalog, "ws one", "session/one")).toBe(
      sessionHref("ws one", "session/one"),
    );
    expect(settingsHref("advanced")).toBe("/settings/advanced");
  });

  it("falls back deterministically when persisted ids are stale", () => {
    expect(
      preferredProductHref(catalog, {
        ...preferences,
        active_workspace_id: "missing",
        active_session_id: "missing",
      }),
    ).toBe("/w/ws%20one/s/session%2Fone");
    expect(defaultWorkspaceHref(catalog, "missing")).toBeNull();
  });
});
