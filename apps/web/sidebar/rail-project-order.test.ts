import { describe, expect, it } from "vitest";

import type { WorkspaceRecord } from "../state/product-types";
import {
  orderWorkspaces,
  reorderProjectOrder,
} from "./rail-project-order";

function workspace(id: string, pinned = false): WorkspaceRecord {
  return {
    id,
    rootPath: `/root/${id}`,
    kind: "folder",
    displayName: id,
    pinned,
    lastOpenedAt: "2026-10-01T00:00:00.000Z",
  };
}

describe("orderWorkspaces", () => {
  it("places remembered ids first in their stored order, rest keep catalog order", () => {
    const workspaces = [workspace("a"), workspace("b"), workspace("c"), workspace("d")];
    expect(
      orderWorkspaces(workspaces, ["c", "a"]).map((item) => item.id),
    ).toEqual(["c", "a", "b", "d"]);
  });

  it("keeps pinned workspaces ahead of any remembered order", () => {
    const workspaces = [workspace("a"), workspace("b"), workspace("pin", true)];
    expect(
      orderWorkspaces(workspaces, ["b", "a"]).map((item) => item.id),
    ).toEqual(["pin", "b", "a"]);
  });

  it("ignores remembered ids that no longer exist", () => {
    const workspaces = [workspace("a"), workspace("b")];
    expect(
      orderWorkspaces(workspaces, ["gone", "b"]).map((item) => item.id),
    ).toEqual(["b", "a"]);
  });
});

describe("reorderProjectOrder", () => {
  it("moves the dragged id before or after the target", () => {
    expect(reorderProjectOrder(["a", "b", "c"], "c", "a", false)).toEqual([
      "c",
      "a",
      "b",
    ]);
    expect(reorderProjectOrder(["a", "b", "c"], "a", "c", true)).toEqual([
      "b",
      "c",
      "a",
    ]);
  });

  it("appends when the target is not in the remembered order", () => {
    expect(reorderProjectOrder(["a"], "b", "gone", false)).toEqual(["a", "b"]);
  });
});
