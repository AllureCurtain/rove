import { expect, test } from "@playwright/test";
import { installMockProductApi } from "./product-api-mock";

const timestamp = "2026-07-26T00:00:00.000Z";

test("project disclosure does not navigate and top new session keeps workspace identity", async ({ page }) => {
  await installMockProductApi(page, {
    workspaces: ["alpha", "beta"].map((id) => ({
      id, canonical_root: `D:/tmp/workbench-${id}`, kind: "folder" as const,
      display_name: id, pinned: false, last_opened_at: timestamp,
      created_at: timestamp, updated_at: timestamp,
    })),
    sessions: [{
      id: "alpha-history", workspace_id: "alpha", title: "Alpha history",
      status: "idle", created_at: timestamp, updated_at: timestamp,
    }],
    activeWorkspaceId: "alpha",
    activeSessionId: "alpha-history",
  });
  await page.goto("/w/alpha/s/alpha-history");
  const sidebar = page.locator(".product-sidebar");
  await expect(sidebar.getByRole("button", { name: "收起 alpha 的会话" })).toBeVisible();
  const originalUrl = page.url();
  await sidebar.getByRole("button", { name: "收起 alpha 的会话" }).click();
  await expect(sidebar.getByRole("button", { name: "展开 alpha 的会话" })).toHaveAttribute("aria-expanded", "false");
  await expect(page).toHaveURL(originalUrl);
  await expect(sidebar.getByText("Alpha history", { exact: true })).toBeHidden();

  await sidebar.getByRole("button", { name: "展开 beta 的会话" }).click();
  await expect(sidebar.getByRole("button", { name: "收起 beta 的会话" })).toHaveAttribute("aria-expanded", "true");
  await expect(page).toHaveURL(originalUrl);
  const actions = sidebar.getByRole("button", { name: "beta 的操作" });
  await actions.click();
  await expect(sidebar.getByRole("menuitem", { name: "固定工作区", exact: true })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(actions).toBeFocused();
  await expect(page).toHaveURL(originalUrl);
  await actions.click();
  await sidebar.getByRole("menuitem", { name: "固定工作区", exact: true }).click();
  await expect(page).toHaveURL(originalUrl);
  await actions.click();
  await expect(sidebar.getByRole("menuitem", { name: "取消固定工作区", exact: true })).toBeVisible();
  await page.keyboard.press("Escape");

  await sidebar.getByRole("searchbox").fill("not-a-loaded-name");
  await expect(sidebar.getByRole("status")).toHaveText("没有匹配的已加载工作区或会话名称。");
  await sidebar.getByRole("searchbox").fill("");
  await expect(sidebar.getByRole("button", { name: "展开 alpha 的会话" })).toBeVisible();

  const created = page.waitForRequest((request) => request.method() === "POST" && new URL(request.url()).pathname.endsWith("/sessions"));
  await sidebar.getByRole("button", { name: "新会话", exact: true }).click();
  const request = await created;
  expect(new URL(request.url()).pathname).toBe("/api/product/sessions");
  expect(request.postDataJSON()).toMatchObject({ workspace_id: "alpha" });
  await expect(page).toHaveURL(/\/w\/alpha\/s\/session-\d+$/);
});

test("top new session without a workspace opens the existing picker", async ({ page }) => {
  const api = await installMockProductApi(page);
  await page.goto("/");
  const create = page.locator(".product-sidebar").getByRole("button", { name: "新会话", exact: true });
  await create.click();
  await expect(page.getByRole("dialog", { name: "打开工作区" })).toBeVisible();
  expect(api.sessions).toHaveLength(0);
  await page.keyboard.press("Escape");
  await expect(create).toBeFocused();
});
