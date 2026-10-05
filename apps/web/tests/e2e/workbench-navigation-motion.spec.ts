import { expect, test } from "@playwright/test";

import {
  completedTranscript,
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
} from "./product-api-mock";

// Desktop viewport: the left rail is a grid column with a draggable handle.
test.use({ viewport: { width: 1440, height: 900 } });

test("left navigation resizes by pointer, keyboard, and persists the preference", async ({
  page,
}) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: {
      [session.id]: completedTranscript(workspace, session, "Question", "Answer"),
    },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(page.getByText("Answer", { exact: true })).toBeVisible();

  const rail = page.locator("aside.product-sidebar");
  const handle = page.getByRole("separator", { name: /导航宽度/ });
  await expect(handle).toBeVisible();
  await expect(rail).not.toHaveAttribute("data-collapsed", "true");

  // The grid column transitions, so the committed width is read from the
  // handle's own value and the rendered width is polled until it settles.
  const railWidth = () => rail.boundingBox().then((box) => Math.round(box!.width));
  await expect.poll(railWidth).toBe(240);

  // Keyboard stepping: one arrow is 16px, Shift is 32px (design §3.3).
  await handle.focus();
  await page.keyboard.press("ArrowRight");
  await expect(handle).toHaveAttribute("aria-valuenow", "256");
  await expect.poll(railWidth).toBe(256);
  await page.keyboard.press("Shift+ArrowRight");
  await expect(handle).toHaveAttribute("aria-valuenow", "288");
  await expect.poll(railWidth).toBe(288);

  // End jumps to the upper bound and Home back to the lower bound.
  await page.keyboard.press("End");
  await expect(handle).toHaveAttribute("aria-valuenow", "360");
  await expect.poll(railWidth).toBe(360);
  await page.keyboard.press("Home");
  await expect(handle).toHaveAttribute("aria-valuenow", "200");
  await expect.poll(railWidth).toBe(200);

  // The choice survives a reload because it is a UI preference.
  await page.reload();
  await expect(page.getByText("Answer", { exact: true })).toBeVisible();
  await expect.poll(railWidth).toBe(200);
});

test("collapsing the rail moves its entries to the header and restores them", async ({
  page,
}) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: {
      [session.id]: completedTranscript(workspace, session, "Question", "Answer"),
    },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(page.getByText("Answer", { exact: true })).toBeVisible();

  await page.getByRole("button", { name: "收起工作区列表" }).click();
  const rail = page.locator("aside.product-sidebar");
  await expect(rail).toHaveAttribute("data-collapsed", "true");
  await expect(rail).toHaveAttribute("aria-hidden", "true");
  await expect(rail).toHaveAttribute("inert", "");
  // The subtree stays mounted rather than unmounting.
  await expect(rail.locator(".session-item").first()).toBeAttached();

  // Collapsed, the header carries the new-session entry and the expand control.
  await expect(
    page.locator(".product-topbar__collapsed").getByRole("button", { name: "新会话" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "展开工作区列表" }).click();
  await expect(rail).not.toHaveAttribute("data-collapsed", "true");
  await expect(rail).not.toHaveAttribute("aria-hidden", "true");
});

test("session list collapse animates then unmounts, and reduced motion is honored", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  const workspace = createMockWorkspace();
  const session = createMockSession();
  const second = createMockSession("session-2", "workspace-1", "Second session");
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session, second],
    transcripts: {
      [session.id]: completedTranscript(workspace, session, "Question", "Answer"),
      [second.id]: completedTranscript(workspace, second, "Second", "Second answer"),
    },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(page.getByText("Answer", { exact: true })).toBeVisible();

  const group = page.locator(".workspace-sessions").first();
  await expect(group).toHaveAttribute("data-expanded", "true");
  await expect(page.locator(".session-item").first()).toBeVisible();

  await page.getByRole("button", { name: new RegExp(`收起.*${workspace.display_name}`) }).click();
  await expect(group).toHaveAttribute("data-expanded", "false");

  // Reduced motion: the global wildcard collapses durations, so the unmount
  // still happens and the row is gone rather than stuck at zero height.
  await expect(group).toBeHidden();
  await expect(page.locator(".session-item")).toHaveCount(0);

  await page.getByRole("button", { name: new RegExp(`展开.*${workspace.display_name}`) }).click();
  await expect(group).toHaveAttribute("data-expanded", "true");
  await expect(page.locator(".session-item").first()).toBeVisible();
});
