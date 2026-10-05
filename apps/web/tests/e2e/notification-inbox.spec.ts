import { expect, test } from "@playwright/test";

import {
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
  type MockSession,
} from "./product-api-mock";

/**
 * Notification inbox (design F5 §6.2/§6.5).
 *
 * The toast stack answers "something just happened"; the inbox answers "what
 * failed while I was not looking", and its rows are the only thing in the shell
 * that can take the reader back to the failure. These cases run in a browser
 * because the surface is a popover anchored in the rail's footer icon row: it
 * has to open above the rail's clipped box, and a row has to be clickable where
 * it is drawn.
 */

const BELL = ".notification-inbox__bell";
const PANEL = ".notification-inbox__panel";
const BADGE = ".notification-inbox__badge";

/**
 * A running session keeps the background poll alive for the whole test, which is
 * what notices a failure the reader did not cause.
 */
function keeper(workspaceId: string): MockSession {
  return {
    ...createMockSession("session-keeper", workspaceId, "Keeper session"),
    status: "running",
  };
}

function setStatus(
  api: { sessions: Array<{ id: string; status: string }> },
  sessionId: string,
  status: string,
): void {
  const session = api.sessions.find((item) => item.id === sessionId);
  if (!session) {
    throw new Error(`no mock session ${sessionId}`);
  }
  session.status = status;
}

test("a session that fails in the background waits in the inbox", async ({ page }) => {
  const workspace = createMockWorkspace();
  const active = createMockSession("session-active", workspace.id, "Active session");
  const background: MockSession = {
    ...createMockSession("session-background", workspace.id, "Background session"),
    status: "running",
  };
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [active, background, keeper(workspace.id)],
    activeWorkspaceId: workspace.id,
    activeSessionId: active.id,
  });

  await page.goto(`/w/${workspace.id}/s/${active.id}`);
  await expect(page.getByRole("heading", { name: "Active session" })).toBeVisible();

  // Nothing has failed yet, so the bell carries no badge.
  await expect(page.locator(BADGE)).toHaveCount(0);

  setStatus(api, background.id, "error");

  // The badge counts unread failures, and the bell says so where it is read.
  await expect(page.locator(BADGE)).toHaveText("1");
  await expect(page.locator(BELL)).toHaveAttribute("aria-label", "打开通知收件箱（1 条未读）");
  await expect(page.locator(PANEL)).toHaveCount(0);

  await page.locator(BELL).click();
  const panel = page.locator(PANEL);
  await expect(panel).toBeVisible();
  await expect(panel).toContainText("通知收件箱");
  await expect(panel).toContainText("Background session");
  await expect(panel).toContainText("执行失败");
  // The row carries the failure's timestamp and where it can take the reader.
  await expect(panel.locator("time")).toHaveAttribute("dateTime", /^\d{4}-\d{2}-\d{2}T/u);
  await expect(panel).toContainText("打开会话");
  // Reading the list is what clears the badge.
  await expect(page.locator(BADGE)).toHaveCount(0);
});

test("an inbox row takes the reader to the session that failed", async ({ page }) => {
  const workspace = createMockWorkspace();
  const active = createMockSession("session-active", workspace.id, "Active session");
  const background: MockSession = {
    ...createMockSession("session-background", workspace.id, "Background session"),
    status: "running",
  };
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [active, background, keeper(workspace.id)],
    activeWorkspaceId: workspace.id,
    activeSessionId: active.id,
  });

  await page.goto(`/w/${workspace.id}/s/${active.id}`);
  await expect(page.getByRole("heading", { name: "Active session" })).toBeVisible();
  setStatus(api, background.id, "error");
  await expect(page.locator(BADGE)).toHaveText("1");

  await page.locator(BELL).click();
  // The row is inside the shell frame, where the v2 tokens live, and it is drawn
  // outside the rail's clipped box; a clipped row could not be clicked here.
  await expect(page.locator(`.product-app-frame ${PANEL}`)).toBeVisible();
  await page.locator("button.notification-inbox__row").first().click();

  await expect(page.getByRole("heading", { name: "Background session" })).toBeVisible();
  // Navigating closes the list: the reader is looking at the failure itself.
  await expect(page.locator(PANEL)).toHaveCount(0);
});

test("a row whose session is gone says so instead of closing on nothing", async ({ page }) => {
  const workspace = createMockWorkspace();
  const active = createMockSession("session-active", workspace.id, "Active session");
  const background: MockSession = {
    ...createMockSession("session-background", workspace.id, "Background session"),
    status: "running",
  };
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [active, background, keeper(workspace.id)],
    activeWorkspaceId: workspace.id,
    activeSessionId: active.id,
  });

  await page.goto(`/w/${workspace.id}/s/${active.id}`);
  await expect(page.getByRole("heading", { name: "Active session" })).toBeVisible();
  setStatus(api, background.id, "error");
  await expect(page.locator(BADGE)).toHaveText("1");

  // The session left the catalog after its failure was noticed — the mock serves
  // the live array, so removing it is what the shell reads on the next refresh.
  // Waiting for the rail to drop the row is what makes "the catalog no longer
  // has it" true on the client side rather than only on the mock's.
  const railRow = page.locator(".session-item").filter({ hasText: "Background session" });
  await expect(railRow).toHaveCount(1);
  api.sessions.splice(
    api.sessions.findIndex((session) => session.id === background.id),
    1,
  );
  await expect(railRow).toHaveCount(0);

  await page.locator(BELL).click();
  await page.locator("button.notification-inbox__row").first().click();

  const panel = page.locator(PANEL);
  await expect(panel).toContainText("这条通知指向的会话已不在目录中。");
  // The reader is still where they were looking; nothing navigated.
  await expect(page.getByRole("heading", { name: "Active session" })).toBeVisible();
});

test("the list stays on screen at a phone width", async ({ page }) => {
  // The bell lives in the rail footer; on a phone that means inside the
  // navigation drawer, and its panel is a fixed bottom sheet. Measured
  // geometry, not visibility: a clipped panel still reports as visible.
  await page.setViewportSize({ width: 390, height: 844 });
  const workspace = createMockWorkspace();
  const session = createMockSession("session-active", workspace.id, "Active session");
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });

  // The dev server's error overlay mounts a fixed-position hit target at the
  // viewport's bottom-left corner — exactly where the open drawer's footer icon
  // row is. It is tooling chrome, not part of the shell under test, and a
  // stylesheet cannot reach inside its portal, so the node is removed.
  await page.addInitScript(() => {
    const removeOverlay = () => {
      for (const el of document.querySelectorAll("nextjs-portal")) el.remove();
    };
    removeOverlay();
    new MutationObserver(removeOverlay).observe(document, {
      childList: true,
      subtree: true,
    });
  });
  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(page.getByRole("heading", { name: "Active session" })).toBeVisible();
  await page.getByRole("button", { name: "展开工作区列表" }).click();
  await page.locator(BELL).click();

  const box = await page.locator(PANEL).boundingBox();
  expect(box).not.toBeNull();
  expect(box!.x).toBeGreaterThanOrEqual(0);
  expect(box!.x + box!.width).toBeLessThanOrEqual(390);
  // The heading is the first thing a left-shifted panel would push out.
  const heading = await page.locator(".notification-inbox__head h2").boundingBox();
  expect(heading!.x).toBeGreaterThanOrEqual(0);
});

test("a failed provider test waits in the inbox with a way back", async ({ page }) => {
  const workspace = createMockWorkspace();
  const session = createMockSession("session-active", workspace.id, "Active session");
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  await page.route("/api/providers/test", async (route) => {
    // The probe outlives the panel that started it, so its result can only be
    // reported by a notification.
    await new Promise((resolve) => setTimeout(resolve, 1_500));
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        status: "error",
        provider: "gateway.test",
        provider_type: "openai",
        api_base: "https://gateway.test/v1",
        key_env: "GATEWAY_API_KEY",
        key_present: true,
        error: "connection refused",
      }),
    });
  });

  await page.goto("/settings/providers");
  await page.getByRole("button", { name: "测试连接" }).click();
  await page.locator(".settings-nav").getByRole("button", { name: "通用" }).click();
  await expect(page.getByRole("heading", { name: "通用" })).toBeVisible();

  await expect(page.locator(BADGE)).toHaveText("1");
  await page.locator(BELL).click();
  const panel = page.locator(PANEL);
  await expect(panel).toContainText("连接失败");
  // The row knows the panel that can fix it, not just that something broke.
  await expect(panel).toContainText("打开设置");

  await page.locator("button.notification-inbox__row").first().click();
  // The row lands on the section that can fix the profile, not on Settings in
  // general: the navigation marks the providers entry as the current page.
  await expect(
    page.locator(".settings-nav").getByRole("button", { name: "模型服务" }),
  ).toHaveAttribute("aria-current", "page");
  await expect(page.locator(PANEL)).toHaveCount(0);
});

test("an inbox with nothing in it says so", async ({ page }) => {
  const workspace = createMockWorkspace();
  const session = createMockSession("session-active", workspace.id, "Active session");
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(page.getByRole("heading", { name: "Active session" })).toBeVisible();

  await page.locator(BELL).click();
  const panel = page.locator(PANEL);
  await expect(panel).toContainText("这次打开页面以来还没有失败。");
  // The list is bounded and lives in this page only; the footer states both.
  await expect(panel).toContainText("内存中只保留最近 50 条");
  await expect(panel.getByRole("button", { name: "清空" })).toBeDisabled();

  // A click outside dismisses it without touching the conversation.
  await page.locator(".conversation-topbar .ct-title").click();
  await expect(panel).toHaveCount(0);
});
