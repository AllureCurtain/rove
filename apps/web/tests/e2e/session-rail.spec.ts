import { expect, test, type Page } from "@playwright/test";

import {
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
} from "./product-api-mock";

/**
 * Session rail structure and row actions (design §7).
 *
 * The rail renders one session in two places — the flat Sessions zone and its
 * project group — so row locators always scope or pick `.first()`; the counts
 * double by design.
 */
const WORKSPACE = createMockWorkspace();

/** Row's ⋯ menu: revealed by hovering the row; the portaled menu carries
 * `.session-row-menu` (its aria-label is the title's or the batch count's). */
async function openRowMenu(page: Page, title: string) {
  const branch = page
    .locator(".session-branch")
    .filter({ hasText: title })
    .first();
  await branch.hover();
  await branch
    .getByRole("button", { name: `${title} 的操作` })
    .click();
  return page.locator(".session-row-menu");
}

test("zones, brand, version and the pinned shelf", async ({ page }) => {
  const session = createMockSession("session-1", WORKSPACE.id, "Pinned candidate");
  await installMockProductApi(page, {
    workspaces: [WORKSPACE],
    sessions: [session],
    activeWorkspaceId: WORKSPACE.id,
    activeSessionId: session.id,
  });
  await page.goto(`/w/${WORKSPACE.id}/s/${session.id}`);

  const rail = page.locator("aside.product-sidebar");
  await expect(rail.getByText("Rove", { exact: true })).toBeVisible();
  await expect(rail.locator(".product-sidebar__version")).toContainText("v0.1.0");
  // Zone headings exist; the pinned shelf only renders when non-empty (§7.1).
  await expect(
    rail.getByRole("region", { name: "会话" }),
  ).toBeVisible();
  await expect(
    rail.getByRole("region", { name: "项目" }),
  ).toBeVisible();
  await expect(rail.getByRole("region", { name: "置顶" })).toHaveCount(0);

  // Pin via the row menu and the row lands in the pinned shelf too.
  const menu = await openRowMenu(page, "Pinned candidate");
  await menu.getByRole("menuitem", { name: "置顶会话" }).click();
  const pinnedZone = rail.getByRole("region", { name: "置顶" });
  await expect(pinnedZone).toBeVisible();
  await expect(
    pinnedZone.locator(".session-item").filter({ hasText: "Pinned candidate" }),
  ).toHaveCount(1);

  // Unpin removes the shelf entirely.
  await openRowMenu(page, "Pinned candidate");
  await page.getByRole("menuitem", { name: "取消置顶" }).click();
  await expect(rail.getByRole("region", { name: "置顶" })).toHaveCount(0);
});

test("archiving moves a session to the Archived group and restore brings it back", async ({
  page,
}) => {
  const session = createMockSession("session-1", WORKSPACE.id, "Old session");
  const keeper = createMockSession("session-2", WORKSPACE.id, "Keeper session");
  const api = await installMockProductApi(page, {
    workspaces: [WORKSPACE],
    sessions: [session, keeper],
    activeWorkspaceId: WORKSPACE.id,
    activeSessionId: keeper.id,
  });
  await page.goto(`/w/${WORKSPACE.id}/s/${keeper.id}`);

  const menu = await openRowMenu(page, "Old session");
  await menu.getByRole("menuitem", { name: "归档" }).click();

  // The row leaves the live zones and appears once, dimmed, under 已归档.
  const archivedGroup = page.locator(".rail-time-group", { hasText: "已归档" });
  await expect(archivedGroup).toBeVisible();
  const archivedRow = archivedGroup.locator(".session-item").filter({
    hasText: "Old session",
  });
  await expect(archivedRow).toHaveCount(1);
  await expect(archivedRow).toHaveAttribute("data-archived", "true");
  // Archived rows leave the project group and live lists.
  const project = page.locator(".workspace-group");
  await expect(
    project.locator(".session-item").filter({ hasText: "Old session" }),
  ).toHaveCount(0);
  await expect.poll(
    () => api.sessions.find((item) => item.id === session.id)?.status,
  ).toBe("archived");

  // Restore returns it to the time groups and the project group.
  const archivedMenu = await openRowMenu(page, "Old session");
  await archivedMenu.getByRole("menuitem", { name: "还原" }).click();
  await expect(
    page.locator(".rail-time-group", { hasText: "已归档" }),
  ).toHaveCount(0);
  await expect(
    project.locator(".session-item").filter({ hasText: "Old session" }),
  ).toHaveCount(1);
  await expect.poll(
    () => api.sessions.find((item) => item.id === session.id)?.status,
  ).toBe("idle");
});

test("the row menu renames a session inline and the title persists", async ({
  page,
}) => {
  const session = createMockSession("session-1", WORKSPACE.id, "Old title");
  const api = await installMockProductApi(page, {
    workspaces: [WORKSPACE],
    sessions: [session],
    activeWorkspaceId: WORKSPACE.id,
    activeSessionId: session.id,
  });
  await page.goto(`/w/${WORKSPACE.id}/s/${session.id}`);

  const menu = await openRowMenu(page, "Old title");
  await menu.getByRole("menuitem", { name: "重命名会话" }).click();
  const editor = page.locator(".session-item--editing input").first();
  await editor.fill("Renamed session");
  await editor.press("Enter");

  await expect(
    page.locator(".session-item").filter({ hasText: "Renamed session" }).first(),
  ).toBeVisible();
  await expect.poll(
    () => api.sessions.find((item) => item.id === session.id)?.title,
  ).toBe("Renamed session");
});

test("ctrl-click multi-select drives the batch archive and armed batch delete", async ({
  page,
}) => {
  const first = createMockSession("session-1", WORKSPACE.id, "Session one");
  const second = createMockSession("session-2", WORKSPACE.id, "Session two");
  const keeper = createMockSession("session-3", WORKSPACE.id, "Keeper session");
  const api = await installMockProductApi(page, {
    workspaces: [WORKSPACE],
    sessions: [first, second, keeper],
    activeWorkspaceId: WORKSPACE.id,
    activeSessionId: keeper.id,
  });
  await page.goto(`/w/${WORKSPACE.id}/s/${keeper.id}`);

  // Scope to the project group so each row resolves once.
  const project = page.locator(".workspace-group");
  const one = project.locator(".session-item").filter({ hasText: "Session one" });
  const two = project.locator(".session-item").filter({ hasText: "Session two" });
  await one.click({ modifiers: ["Control"] });
  await two.click({ modifiers: ["Control"] });
  await expect(one).toHaveAttribute("data-multi-selected", "true");
  await expect(two).toHaveAttribute("data-multi-selected", "true");
  // A selection click is not a navigation.
  await expect(page).toHaveURL(`/w/${WORKSPACE.id}/s/${keeper.id}`);

  // The ⋯ menu of a selected row speaks for the batch.
  const menu = await openRowMenu(page, "Session one");
  await expect(
    menu.getByRole("menuitem", { name: "归档 2 个会话" }),
  ).toBeVisible();
  await menu.getByRole("menuitem", { name: "归档 2 个会话" }).click();
  await expect.poll(
    () =>
      api.sessions.filter((item) => item.status === "archived").length,
  ).toBe(2);

  // Re-select the archived pair from the 已归档 group and delete them with
  // the two-step armed item.
  const archived = page.locator(".rail-time-group", { hasText: "已归档" });
  await archived
    .locator(".session-item")
    .filter({ hasText: "Session one" })
    .click({ modifiers: ["Control"] });
  await archived
    .locator(".session-item")
    .filter({ hasText: "Session two" })
    .click({ modifiers: ["Control"] });
  const batchMenu = await openRowMenu(page, "Session one");
  await batchMenu
    .getByRole("menuitem", { name: "删除 2 个会话" })
    .click();
  // First click arms, second commits.
  await batchMenu
    .getByRole("menuitem", { name: "确认删除 2 个会话？" })
    .click();
  await expect.poll(() => api.sessions).toHaveLength(1);
  expect(api.sessions[0]?.id).toBe(keeper.id);
});

test("branch forks an idle session at its latest turn", async ({ page }) => {
  const session = {
    ...createMockSession("session-1", WORKSPACE.id, "Finished work"),
    runtime_binding: {
      ordinal: 1,
      runtime_session_id: "runtime-session-1",
      latest_job_id: "job-1",
      latest_run_id: "run-1",
    },
  };
  const api = await installMockProductApi(page, {
    workspaces: [WORKSPACE],
    sessions: [session],
    activeWorkspaceId: WORKSPACE.id,
    activeSessionId: session.id,
  });
  await page.goto(`/w/${WORKSPACE.id}/s/${session.id}`);

  const menu = await openRowMenu(page, "Finished work");
  await menu.getByRole("menuitem", { name: "创建分支" }).click();
  await expect.poll(() => api.forkRequests).toHaveLength(1);
  expect(api.forkRequests[0]).toMatchObject({ sessionId: session.id });
  // The fork lands as a new row.
  await expect(
    page.locator(".session-item").filter({ hasText: /派生|Fork/ }).first(),
  ).toBeVisible();
});

test("a running session cannot be branched and the menu says why", async ({
  page,
}) => {
  const session = {
    ...createMockSession("session-1", WORKSPACE.id, "Busy session"),
    status: "running" as const,
    runtime_binding: {
      ordinal: 1,
      runtime_session_id: "runtime-session-1",
      latest_job_id: "job-1",
      latest_run_id: "run-1",
    },
  };
  await installMockProductApi(page, {
    workspaces: [WORKSPACE],
    sessions: [session],
    activeWorkspaceId: WORKSPACE.id,
    activeSessionId: session.id,
  });
  await page.goto(`/w/${WORKSPACE.id}/s/${session.id}`);

  const menu = await openRowMenu(page, "Busy session");
  const branch = menu.getByRole("menuitem", { name: "创建分支" });
  await expect(branch).toBeDisabled();
  await expect(branch).toHaveAttribute(
    "title",
    "需要有已完成的轮次且会话空闲才能创建分支",
  );
});

test("project rename writes through and the project menu names it", async ({
  page,
}) => {
  const session = createMockSession("session-1", WORKSPACE.id, "Work session");
  const api = await installMockProductApi(page, {
    workspaces: [WORKSPACE],
    sessions: [session],
    activeWorkspaceId: WORKSPACE.id,
    activeSessionId: session.id,
  });
  await page.goto(`/w/${WORKSPACE.id}/s/${session.id}`);

  const rail = page.locator("aside.product-sidebar");
  await rail
    .locator(".workspace-group")
    .getByRole("button", { name: "rove-shell-demo 的操作" })
    .click();
  await page.getByRole("menuitem", { name: "重命名项目" }).click();
  const editor = rail.locator(".workspace-group__rename input");
  await editor.fill("renamed-project");
  await editor.press("Enter");

  await expect(
    rail.locator(".workspace-group__button").first(),
  ).toContainText("renamed-project");
  await expect.poll(() => api.workspaces[0]?.display_name).toBe(
    "renamed-project",
  );
});
