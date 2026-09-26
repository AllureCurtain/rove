import { expect, test } from "@playwright/test";

import {
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
  type MockSession,
} from "./product-api-mock";

/**
 * Toasts (design F5).
 *
 * Only work that happens outside the reader's view earns a toast. A session
 * that fails while background polling runs says so once, not once per poll and
 * not for a failure that was already there when the page opened. The provider
 * connection test reports inline while its panel is mounted, and through a
 * toast once that panel is gone.
 */

const TOAST = ".toast";

/**
 * A test that makes a background session fail: the mock serves the status the
 * test writes, and the next background poll is what the shell notices.
 */
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

test("a session that fails in the background raises one toast", async ({ page }) => {
  const workspace = createMockWorkspace();
  const active = createMockSession("session-active", workspace.id, "Active session");
  const background = createMockSession(
    "session-background",
    workspace.id,
    "Background session",
  );
  // The poll runs while any session claims to be running, so `keeper` keeps it
  // alive for the whole test.
  const keeper = createMockSession("session-keeper", workspace.id, "Keeper session");
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [
      active,
      { ...background, status: "running" },
      { ...keeper, status: "running" },
    ],
    activeWorkspaceId: workspace.id,
    activeSessionId: active.id,
  });

  await page.goto(`/w/${workspace.id}/s/${active.id}`);
  await expect(page.getByRole("heading", { name: "Active session" })).toBeVisible();
  const readsBefore = api.sessionListReads;

  setStatus(api, background.id, "error");

  const toast = page.locator(TOAST);
  await expect(toast).toHaveCount(1);
  await expect(toast).toContainText("Background session");
  await expect(toast).toContainText("执行失败");
  await expect(toast).toHaveAttribute("role", "alert");
  // Polling is what noticed, not a page reload.
  expect(api.sessionListReads).toBeGreaterThan(readsBefore);

  // The stack is inside the shell frame, where the v2 tokens live, and the
  // viewport never covers a click target.
  await expect(page.locator(`.product-app-frame ${TOAST}`)).toHaveCount(1);
  await expect(page.locator(".toast-viewport")).toHaveCSS("pointer-events", "none");
});

test("the same session failing twice inside the window says it once", async ({ page }) => {
  const workspace = createMockWorkspace();
  const active = createMockSession("session-active", workspace.id, "Active session");
  const background = createMockSession(
    "session-background",
    workspace.id,
    "Background session",
  );
  const fresh = createMockSession("session-fresh", workspace.id, "Fresh session");
  const keeper = createMockSession("session-keeper", workspace.id, "Keeper session");
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [
      active,
      { ...background, status: "running" },
      { ...fresh, status: "running" },
      { ...keeper, status: "running" },
    ],
    activeWorkspaceId: workspace.id,
    activeSessionId: active.id,
  });

  await page.goto(`/w/${workspace.id}/s/${active.id}`);
  await expect(page.getByRole("heading", { name: "Active session" })).toBeVisible();
  const toast = page.locator(TOAST);
  const backgroundRow = page
    .locator(".session-item")
    .filter({ hasText: "Background session" });

  // First failure: the reader is told, and can dismiss it like any other toast.
  setStatus(api, background.id, "error");
  await expect(toast).toHaveCount(1);
  await expect(toast).toContainText("Background session");
  await toast.locator(".toast__dismiss").click();
  await expect(toast).toHaveCount(0);

  // It recovers, which the sidebar shows, so the next failure is a real
  // transition rather than a status that never changed.
  setStatus(api, background.id, "running");
  await expect(backgroundRow).toContainText("运行中");

  // Within the dedupe window the same session fails again; a second session
  // fails in the same poll, and that one is new.
  setStatus(api, background.id, "error");
  setStatus(api, fresh.id, "error");

  await expect(toast).toHaveCount(1, { timeout: 15_000 });
  await expect(toast).toContainText("Fresh session");
  await expect(page.locator(TOAST).filter({ hasText: "Background session" })).toHaveCount(0);
});

test("a failure that predates the page load is not announced", async ({ page }) => {
  const workspace = createMockWorkspace();
  const active = createMockSession("session-active", workspace.id, "Active session");
  const broken = createMockSession("session-broken", workspace.id, "Broken session");
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [active, { ...broken, status: "error" }],
    activeWorkspaceId: workspace.id,
    activeSessionId: active.id,
  });

  await page.goto(`/w/${workspace.id}/s/${active.id}`);
  await expect(page.getByRole("heading", { name: "Active session" })).toBeVisible();
  await page.waitForTimeout(1_500);
  await expect(page.locator(TOAST)).toHaveCount(0);
});

test("the active session reports its own failure inline", async ({ page }) => {
  const workspace = createMockWorkspace();
  const session = createMockSession("session-active", workspace.id, "Active session");
  const keeper = createMockSession("session-keeper", workspace.id, "Keeper session");
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    // Both sessions run, so a poll happens; the one the reader is looking at
    // then fails. The conversation shows that, so no toast repeats it.
    sessions: [
      { ...session, status: "running" },
      { ...keeper, status: "running" },
    ],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(page.getByRole("heading", { name: "Active session" })).toBeVisible();
  setStatus(api, session.id, "error");
  await expect(
    page.locator(".session-item").filter({ hasText: "Active session" }),
  ).toContainText("失败");
  await page.waitForTimeout(3_000);
  await expect(page.locator(TOAST)).toHaveCount(0);
});

test("a successful fork is announced", async ({ page }) => {
  const workspace = createMockWorkspace();
  const session: MockSession = {
    ...createMockSession("session-active", workspace.id, "Active session"),
    // A fork continues a completed turn, so the session needs its latest run.
    runtime_binding: {
      ordinal: 1,
      runtime_session_id: "runtime-session-1",
      latest_job_id: "job-1",
      latest_run_id: "run-1",
    },
  };
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await page.getByRole("button", { name: "Fork", exact: true }).click();

  const toast = page.locator(TOAST);
  await expect(toast).toHaveCount(1);
  await expect(toast).toContainText("已创建分支会话");
  await expect(toast).toContainText("Fork of Active session");
  await expect(toast).toHaveAttribute("role", "status");
  await expect(toast).toHaveClass(/toast--success/u);

  // The success kind leaves on its own after four seconds.
  await expect(toast).toHaveCount(0, { timeout: 8_000 });
});

test("reduced motion shows toasts without the slide", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  const workspace = createMockWorkspace();
  const session: MockSession = {
    ...createMockSession("session-active", workspace.id, "Active session"),
    runtime_binding: {
      ordinal: 1,
      runtime_session_id: "runtime-session-1",
      latest_job_id: "job-1",
      latest_run_id: "run-1",
    },
  };
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await page.getByRole("button", { name: "Fork", exact: true }).click();

  const toast = page.locator(TOAST);
  await expect(toast).toHaveCount(1);
  await expect(page.locator(".toast-viewport")).toHaveAttribute("data-motion", "reduced");
  await expect(toast).toHaveCSS("animation-name", "none");
});

test("a provider test whose panel is gone still reports", async ({ page }) => {
  const workspace = createMockWorkspace();
  const session = createMockSession("session-active", workspace.id, "Active session");
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  await page.route("/api/providers/test", async (route) => {
    // Outlive the panel that started it.
    await new Promise((resolve) => setTimeout(resolve, 1_500));
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        status: "ok",
        provider: "gateway.test",
        provider_type: "openai",
        api_base: "https://gateway.test/v1",
        key_env: "GATEWAY_API_KEY",
        key_present: true,
        model: "relay/model-a",
        model_present: true,
        models_count: 2,
      }),
    });
  });

  await page.goto("/settings/providers");
  await page.getByRole("button", { name: "测试连接" }).click();
  // Leaving the providers section unmounts the panel and its inline result.
  await page.locator(".settings-nav").getByRole("button", { name: "通用" }).click();
  await expect(page.getByRole("heading", { name: "通用" })).toBeVisible();

  const toast = page.locator(TOAST);
  await expect(toast).toHaveCount(1);
  await expect(toast).toContainText("连接成功");
  await expect(toast).toHaveAttribute("role", "status");
});
