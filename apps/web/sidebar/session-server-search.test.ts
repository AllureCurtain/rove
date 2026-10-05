import { describe, expect, it } from "vitest";

import type { SessionRecord } from "../state/product-types";
import {
  resolveSessionRows,
  serverSearchQuery,
} from "./session-server-search";

function session(id: string, title: string): SessionRecord {
  return { id, title } as SessionRecord;
}

const LOCAL = [session("a", "Alpha work"), session("b", "Beta review")];

describe("serverSearchQuery", () => {
  it("trims the query and reports a blank one as nothing to ask", () => {
    expect(serverSearchQuery("  alpha ")).toBe("alpha");
    expect(serverSearchQuery("   ")).toBeNull();
    expect(serverSearchQuery("")).toBeNull();
  });
});

describe("resolveSessionRows", () => {
  it("restores every loaded row while the query is empty", () => {
    const resolved = resolveSessionRows({
      localSessions: LOCAL,
      serverMatches: null,
      query: "",
      workspaceMatches: false,
    });
    expect(resolved.sessions).toBe(LOCAL);
    expect(resolved.visible).toBe(true);
  });

  it("keeps a workspace whose own name matched, even with no sessions", () => {
    const resolved = resolveSessionRows({
      localSessions: [],
      serverMatches: [],
      query: "rove",
      workspaceMatches: true,
    });
    expect(resolved.sessions).toEqual([]);
    expect(resolved.visible).toBe(true);
  });

  it("falls back to the local substring while the server has not answered", () => {
    const resolved = resolveSessionRows({
      localSessions: LOCAL,
      serverMatches: null,
      query: "beta",
      workspaceMatches: false,
    });
    expect(resolved.sessions.map((item) => item.id)).toEqual(["b"]);
    expect(resolved.visible).toBe(true);
  });

  it("hides a workspace when neither the server nor the local filter matched", () => {
    const resolved = resolveSessionRows({
      localSessions: LOCAL,
      serverMatches: null,
      query: "gamma",
      workspaceMatches: false,
    });
    expect(resolved.sessions).toEqual([]);
    expect(resolved.visible).toBe(false);
  });

  it("prefers the server's rows, including sessions the catalog never loaded", () => {
    const fromServer = [session("c", "Release checklist")];
    const resolved = resolveSessionRows({
      localSessions: LOCAL,
      serverMatches: fromServer,
      query: "checklist",
      workspaceMatches: false,
    });
    expect(resolved.sessions).toBe(fromServer);
    expect(resolved.visible).toBe(true);
  });

  it("trusts an empty server answer as 'no match' instead of re-filtering locally", () => {
    const resolved = resolveSessionRows({
      localSessions: LOCAL,
      serverMatches: [],
      query: "alpha",
      workspaceMatches: false,
    });
    expect(resolved.sessions).toEqual([]);
    expect(resolved.visible).toBe(false);
  });
});
