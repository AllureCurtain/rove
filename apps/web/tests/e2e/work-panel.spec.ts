import { expect, test, type Page } from "@playwright/test";

import { createMockSession, createMockWorkspace, installMockProductApi } from "./product-api-mock";

/**
 * Design §8 work-panel contract: closed by default, a persisted open
 * preference, a per-session strip whose order and selection survive reloads,
 * keyboard reorder, and a maximize mode that gives the panel the whole client
 * area beside the rail while the resize handle sits out.
 */

async function seed(page: Page) {
  const workspace = createMockWorkspace();
  const a = createMockSession("session-a", workspace.id, "Session A");
  const b = createMockSession("session-b", workspace.id, "Session B");
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [a, b],
    activeWorkspaceId: workspace.id,
    activeSessionId: a.id,
  });
  await page.goto(`/w/${workspace.id}/s/${a.id}`);
  return { workspace, a, b };
}

async function openPanel(page: Page) {
  await page.getByRole("button", { name: "展开详情面板" }).click();
  await expect(
    page.locator("aside.product-inspector"),
  ).not.toHaveAttribute("data-collapsed", "true");
}

/** Opens a tab through the `+` launcher. */
async function openTab(page: Page, name: string) {
  await page.getByRole("button", { name: "打开一个页签", exact: true }).click();
  await page
    .getByRole("tabpanel")
    .getByRole("button", { name, exact: true })
    .click();
}

async function tabOrder(page: Page): Promise<string[]> {
  return page
    .getByRole("tablist", { name: "详情页签" })
    .getByRole("tab")
    .allTextContents();
}

test("panel starts closed and remembers the open preference across reloads", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await seed(page);
  const inspector = page.locator("aside.product-inspector");

  // Closed by default (design §8): the subtree stays mounted but inert.
  await expect(inspector).toHaveAttribute("data-collapsed", "true");
  await expect(inspector).toHaveAttribute("aria-hidden", "true");

  await openPanel(page);
  await page.reload();
  // A deliberate open is the user's preference, so a reload restores it.
  await expect(inspector).not.toHaveAttribute("data-collapsed", "true");

  // The floating toggle shares this label, so scope to the panel itself.
  await inspector.getByRole("button", { name: "收起详情面板", exact: true }).click();
  await expect(inspector).toHaveAttribute("data-collapsed", "true");
  await page.reload();
  await expect(inspector).toHaveAttribute("data-collapsed", "true");
});

test("tab strip persists its order per session and reorders with Alt+arrows", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await seed(page);
  await openPanel(page);

  await openTab(page, "文件");
  await openTab(page, "运行");
  const strip = page.getByRole("tablist", { name: "详情页签" });
  expect(await tabOrder(page)).toEqual(["变更", "文件", "运行"]);

  // Alt+ArrowLeft reorders the focused tab and keeps focus on it (design §8).
  const statusTab = strip.getByRole("tab", { name: "运行", exact: true });
  await statusTab.focus();
  await statusTab.press("Alt+ArrowLeft");
  expect(await tabOrder(page)).toEqual(["变更", "运行", "文件"]);
  await expect(statusTab).toBeFocused();
  await expect(statusTab).toHaveAttribute("aria-selected", "true");

  // The strip is session state: a reload restores exactly what was left. The
  // stored strip applies in a mount effect, so poll rather than read one frame.
  await page.reload();
  await expect.poll(() => tabOrder(page)).toEqual(["变更", "运行", "文件"]);
  await expect(
    page
      .getByRole("tablist", { name: "详情页签" })
      .getByRole("tab", { name: "运行", exact: true }),
  ).toHaveAttribute("aria-selected", "true");

  // Session isolation (design §8): another session keeps its own strip.
  await page
    .locator(".workspace-group")
    .getByRole("button", { name: /^Session B(?:,|$)/ })
    .click();
  await expect(page.getByRole("heading", { name: "Session B" })).toBeVisible();
  await expect.poll(() => tabOrder(page)).toEqual(["变更"]);

  // ...and switching back restores the first session's strip, not a copy.
  await page
    .locator(".workspace-group")
    .getByRole("button", { name: /^Session A(?:,|$)/ })
    .click();
  await expect(page.getByRole("heading", { name: "Session A" })).toBeVisible();
  await expect.poll(() => tabOrder(page)).toEqual(["变更", "运行", "文件"]);
});

test("a pointer drag reorders the strip", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await seed(page);
  await openPanel(page);
  await openTab(page, "文件");
  await openTab(page, "运行");

  const strip = page.getByRole("tablist", { name: "详情页签" });
  const status = await strip
    .getByRole("tab", { name: "运行", exact: true })
    .boundingBox();
  const changes = await strip
    .getByRole("tab", { name: "变更", exact: true })
    .boundingBox();
  expect(status && changes).toBeTruthy();

  // Drag 运行 past 变更's midpoint: the drop lands before it.
  const y = status!.y + status!.height / 2;
  await page.mouse.move(status!.x + status!.width / 2, y);
  await page.mouse.down();
  for (let step = 1; step <= 8; step += 1) {
    await page.mouse.move(status!.x + status!.width / 2 - step * 20, y);
  }
  await page.mouse.move(changes!.x + changes!.width / 2 - 4, y);
  await page.mouse.up();

  expect(await tabOrder(page)).toEqual(["运行", "变更", "文件"]);
});

test("maximize gives the panel the client area and disables the separator", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await seed(page);
  await openPanel(page);

  const separator = page.getByRole("separator", { name: /面板宽度/ });
  await expect(separator).toBeEnabled();

  await page
    .getByRole("button", { name: "最大化面板", exact: true })
    .click();
  const inspector = page.locator("aside.product-inspector");
  await expect(inspector).toHaveAttribute("data-maximized", "true");

  // The conversation column yields entirely (design §8) and the resize handle
  // is disabled while maximized.
  await expect(page.locator(".product-main")).toBeHidden();
  await expect(separator).toHaveAttribute("aria-disabled", "true");
  await expect(separator).toHaveAttribute("tabindex", "-1");
  const box = await inspector.boundingBox();
  const rail = await page.locator(".product-sidebar").boundingBox();
  expect(
    box!.width,
    "the panel takes the whole client area beside the rail",
  ).toBeGreaterThan(1440 - rail!.width - 10);

  await page
    .getByRole("button", { name: "还原面板", exact: true })
    .click();
  await expect(inspector).not.toHaveAttribute("data-maximized", "true");
  await expect(page.locator(".product-main")).toBeVisible();
  await expect(separator).toBeEnabled();
});
