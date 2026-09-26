import { expect, test } from "@playwright/test";

import {
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
} from "./product-api-mock";

/**
 * F11 (design §12): the sidebar asks the server for sessions matching the query
 * while the local catalog stays the offline degradation.
 */

test("a session only the server knows about appears in the search", async ({
  page,
}) => {
  const workspace = createMockWorkspace();
  const loaded = createMockSession("session-loaded", workspace.id, "Alpha work");
  const unloaded = createMockSession(
    "session-unloaded",
    workspace.id,
    "Release checklist",
  );
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [loaded],
    searchOnlySessions: [unloaded],
    activeWorkspaceId: workspace.id,
    activeSessionId: loaded.id,
  });
  await page.goto(`/w/${workspace.id}/s/${loaded.id}`);
  await expect(page.locator(".chat-pane__header h1")).toHaveText("Alpha work");

  const search = page.getByRole("searchbox", { name: "搜索工作区与会话" });
  await search.fill("checklist");

  // Nothing loaded matches, so before the server answers the sidebar says so.
  await expect(page.locator(".sidebar-empty")).toContainText("没有匹配");

  // The server's rows replace the local filter: this session was never part of
  // the catalog, so only a server search can produce it.
  await expect(page.getByRole("button", { name: /Release checklist/ })).toBeVisible();
  expect(api.sessionSearchRequests).toEqual([
    { q: "checklist", limit: "200" },
  ]);

  // Clearing the query restores the loaded list, with no server rows left over.
  await search.fill("");
  await expect(page.getByRole("button", { name: /Alpha work/ })).toBeVisible();
  await expect(page.getByRole("button", { name: /Release checklist/ })).toHaveCount(0);
});

test("a failed server search leaves the local filter in charge", async ({
  page,
}) => {
  const workspace = createMockWorkspace();
  const loaded = createMockSession("session-loaded", workspace.id, "Alpha work");
  const other = createMockSession("session-other", workspace.id, "Beta review");
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [loaded, other],
    sessionSearchFailures: 1,
    activeWorkspaceId: workspace.id,
    activeSessionId: loaded.id,
  });
  await page.goto(`/w/${workspace.id}/s/${loaded.id}`);
  await expect(page.locator(".chat-pane__header h1")).toHaveText("Alpha work");

  await page.getByRole("searchbox", { name: "搜索工作区与会话" }).fill("beta");

  // The refusal is visible, and the loaded rows are still filtered locally.
  await expect(page.locator(".workspace-search-scope")).toContainText(
    "服务端检索不可用",
  );
  await expect(page.getByRole("button", { name: /Beta review/ })).toBeVisible();
  await expect(page.getByRole("button", { name: /Alpha work/ })).toHaveCount(0);
});
