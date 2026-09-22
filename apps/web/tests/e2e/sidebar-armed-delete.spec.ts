import { expect, test } from "@playwright/test";

import { ARMED_DELETE_MS } from "../../shell/use-armed-delete";
import {
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
} from "./product-api-mock";

/**
 * Workspace removal also drops that workspace's sessions from the catalog, so it
 * takes two clicks: the first arms the control and relabels it, the second
 * removes. The arm expires on its own, and a dismissed menu never reopens armed
 * (ported from PI-Desktop's `hooks/use-armed-delete.ts`).
 */
async function openSidebarMenu(page: import("@playwright/test").Page) {
  await page.getByRole("button", { name: /的操作$/ }).first().click();
}

test("removing a workspace needs a second click", async ({ page }) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  await page.goto(`/w/${workspace.id}/s/${session.id}`);

  await openSidebarMenu(page);
  await page.getByRole("menuitem", { name: "从列表移除工作区" }).click();

  // Armed, not removed: the control relabels and keeps the workspace.
  const armed = page.getByRole("menuitem", { name: "确认移除工作区及其会话" });
  await expect(armed).toBeVisible();
  await expect(armed).toHaveAttribute("data-armed", "true");
  expect(api.workspaces).toHaveLength(1);

  await armed.click();
  await expect.poll(() => api.workspaces).toHaveLength(0);
});

test("an armed removal disarms itself and on a dismissed menu", async ({ page }) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  await page.goto(`/w/${workspace.id}/s/${session.id}`);

  // The arm expires by itself, so the row is never one stray click from removal.
  await openSidebarMenu(page);
  await page.getByRole("menuitem", { name: "从列表移除工作区" }).click();
  await expect(
    page.getByRole("menuitem", { name: "确认移除工作区及其会话" }),
  ).toBeVisible();
  await page.waitForTimeout(ARMED_DELETE_MS + 500);
  await expect(
    page.getByRole("menuitem", { name: "从列表移除工作区" }),
  ).toBeVisible();
  await expect(
    page.getByRole("menuitem", { name: "确认移除工作区及其会话" }),
  ).toHaveCount(0);
  expect(api.workspaces).toHaveLength(1);

  // Escape dismisses the menu, and reopening it starts unarmed.
  await page.getByRole("menuitem", { name: "从列表移除工作区" }).click();
  await expect(
    page.getByRole("menuitem", { name: "确认移除工作区及其会话" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await openSidebarMenu(page);
  const reopened = page.getByRole("menuitem", { name: "从列表移除工作区" });
  await expect(reopened).toBeVisible();
  await expect(reopened).not.toHaveAttribute("data-armed", "true");
  expect(api.workspaces).toHaveLength(1);
});
