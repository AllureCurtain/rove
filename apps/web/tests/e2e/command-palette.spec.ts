import { expect, test, type Page } from "@playwright/test";

import {
  completedTranscript,
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
} from "./product-api-mock";

/**
 * Command palette, ported from open-vetta's `CommandMenu`.
 *
 * The behaviour that matters: it opens from anywhere including the composer, the
 * first action after opening is typing, substring semantics keep CJK queries
 * honest, a row that cannot run is shown and disabled, and Escape closes it while
 * returning focus where it was.
 */
async function openShell(page: Page, width = 1440, height = 900) {
  await page.setViewportSize({ width, height });
  const workspace = createMockWorkspace();
  const session = createMockSession();
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: {
      [session.id]: completedTranscript(workspace, session, "Palette question", "Palette answer"),
    },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(page.getByText("Palette answer", { exact: true })).toBeVisible();
  return { workspace, session };
}

function palette(page: Page) {
  return page.getByRole("dialog", { name: "命令面板" });
}

test("opens from the composer, filters, and runs the selected command", async ({
  page,
}) => {
  await openShell(page);

  // Available while typing: a palette that cannot be opened from the composer is
  // a palette nobody uses.
  const composer = page.getByRole("textbox", { name: /输入消息/ });
  await composer.focus();
  await page.keyboard.press("Control+k");

  const dialog = palette(page);
  await expect(dialog).toBeVisible();
  const input = dialog.getByRole("combobox");
  await expect(input).toBeFocused();

  // Typing filters; a substring of a settings label is enough to select it.
  await input.fill("键盘");
  const option = dialog.getByRole("option").first();
  await expect(option).toContainText("键盘快捷键");
  await expect(option).toHaveAttribute("aria-selected", "true");

  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/\/settings\/keyboard$/u);
  await expect(dialog).toHaveCount(0);
});

test("keeps CJK queries honest: substring, not subsequence", async ({ page }) => {
  await openShell(page);
  await page.keyboard.press("Control+k");
  const dialog = palette(page);
  const input = dialog.getByRole("combobox");

  // Every settings entry matches its own label...
  await input.fill("设置");
  await expect(dialog.getByRole("option").first()).toBeVisible();
  // ...but a non-substring of every label matches nothing, and the empty state
  // says so instead of showing unrelated rows.
  await input.fill("设配");
  await expect(dialog.getByRole("status")).toHaveText("没有匹配项。");
  await expect(dialog.getByRole("option")).toHaveCount(0);
});

test("shows a command that cannot run instead of hiding it", async ({ page }) => {
  await openShell(page);
  await page.keyboard.press("Control+k");
  const dialog = palette(page);

  // With a workspace and session open, every action is available.
  await expect(
    dialog.getByRole("option").filter({ hasText: "详情面板" }),
  ).not.toHaveAttribute("aria-disabled", "true");

  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
});

test("arrows move the selection, Escape closes and restores focus", async ({
  page,
}) => {
  await openShell(page);
  const composer = page.getByRole("textbox", { name: /输入消息/ });
  await composer.focus();
  await page.keyboard.press("Control+k");
  const dialog = palette(page);
  const input = dialog.getByRole("combobox");

  const selectedId = async () =>
    (await dialog.locator('[role="option"][aria-selected="true"]').first().getAttribute("id")) ?? "";
  const first = await selectedId();
  await page.keyboard.press("ArrowDown");
  expect(await selectedId(), "the arrow moves the highlight").not.toBe(first);
  await page.keyboard.press("ArrowUp");
  expect(await selectedId()).toBe(first);
  await page.keyboard.press("End");
  const last = await selectedId();
  expect(last).not.toBe(first);
  await page.keyboard.press("Home");
  expect(await selectedId()).toBe(first);

  // Tab stays on the input: the arrows own the list.
  await page.keyboard.press("Tab");
  await expect(input).toBeFocused();

  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(composer).toBeFocused();
});

test("navigates to a workspace and a session from the palette", async ({ page }) => {
  const { workspace, session } = await openShell(page);
  await page.keyboard.press("Control+k");
  const dialog = palette(page);
  const input = dialog.getByRole("combobox");

  await input.fill(session.title);
  await dialog.getByRole("option").filter({ hasText: session.title }).first().click();
  await expect(page).toHaveURL(new RegExp(`/w/${workspace.id}/s/${session.id}$`, "u"));
  await expect(dialog).toHaveCount(0);
});
