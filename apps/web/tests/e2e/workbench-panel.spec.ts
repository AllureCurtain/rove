import { expect, test, type Page } from "@playwright/test";

import {
  MAIN_PANE_MIN_WIDTH,
  WORK_PANEL_DEFAULT_WIDTH,
  WORK_PANEL_MIN_WIDTH,
} from "../../inspector/work-panel-layout";
import { createMockSession, createMockWorkspace, installMockProductApi } from "./product-api-mock";

async function pendingRun(page: Page) {
  const workspace = createMockWorkspace();
  const a = createMockSession("session-a", workspace.id, "Session A");
  const b = createMockSession("session-b", workspace.id, "Session B");
  const api = await installMockProductApi(page, { mode: "approval", workspaces: [workspace],
    sessions: [a, b], activeWorkspaceId: workspace.id, activeSessionId: a.id });
  await page.goto(`/w/${workspace.id}/s/${a.id}`);
  await page.getByRole("textbox", { name: /输入消息/ }).fill("Write a note");
  await page.getByRole("button", { name: "发送", exact: true }).click();
  const inline = page.getByLabel("Conversation").getByRole("alert", { name: "审批", exact: true });
  await expect(inline.getByRole("button", { name: "批准", exact: true })).toBeVisible();
  await expect(inline.getByText("destructive tool requires explicit approval")).toBeVisible();
  return { api, inline, workspace, a, b };
}

for (const decision of ["approve", "reject"] as const) {
  test(`panel and inline share ${decision}, synchronous busy and exact request`, async ({ page }) => {
    const { api, inline } = await pendingRun(page);
    await inline.getByRole("button", { name: "审批详情", exact: true }).click();
    const detail = page.getByRole("region", { name: "审批详情", exact: true });
    await expect(detail.getByText("run-1 / call-approval-1", { exact: false })).toBeVisible();
    await expect(detail.locator("pre")).toHaveText(await inline.locator("pre").innerText());
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const bodies: unknown[] = [];
    await page.route(/\/api\/jobs\/[^/]+\/approvals\/[^/]+$/, async (route) => {
      bodies.push(route.request().postDataJSON());
      await gate;
      // Keep the run running after either decision. Reject must not invent idle.
      const binding = api.jobStarts[0];
      await route.fulfill({ json: { job_id: binding.job_id, run_id: binding.run_id,
        status: "running", event_count: 2, events: [], pending_approvals: [], pending_inputs: [] } });
    });
    const button = detail.getByRole("button", { name: decision === "approve" ? "批准" : "拒绝", exact: true });
    await button.evaluate((element: HTMLButtonElement) => { element.click(); element.click(); });
    await expect(inline.getByRole("button", { name: "批准", exact: true })).toBeDisabled();
    await expect(detail.getByRole("button", { name: "拒绝", exact: true })).toBeDisabled();
    expect(bodies).toEqual([{ decision }]);
    release();
    await expect(detail.getByText(/不表示本窗口批准成功/)).toBeVisible();
    await expect(inline).toHaveCount(0);
    await expect(page.getByRole("button", { name: "停止", exact: true })).toBeVisible();
  });
}

for (const failure of [404, 409, "offline"] as const) {
  test(`approval ${failure} authoritatively reloads without resend or success claim`, async ({ page }) => {
    const { inline } = await pendingRun(page);
    await inline.getByRole("button", { name: "审批详情", exact: true }).click();
    const detail = page.getByRole("region", { name: "审批详情", exact: true });
    let posts = 0;
    let reads = 0;
    await page.route(/\/api\/jobs\/[^/]+\/state$/, async (route) => { reads++; await route.fallback(); });
    await page.route(/\/api\/jobs\/[^/]+\/approvals\/[^/]+$/, async (route) => {
      posts++;
      if (failure === "offline") await route.abort("connectionfailed");
      else await route.fulfill({ status: failure, json: { error: "resolved elsewhere" } });
    });
    await detail.getByRole("button", { name: "拒绝", exact: true }).click();
    await expect(detail.getByRole("alert").filter({ hasText: "Approval outcome not confirmed" })).toBeVisible();
    await expect(inline.getByRole("button", { name: "批准", exact: true })).toBeEnabled();
    expect(posts).toBe(1);
    expect(reads).toBeGreaterThan(0);
    await expect(page.getByText("Approved write completed")).toHaveCount(0);
  });
}

test("refresh restores pending request; panel width is keyboard bounded and selection session-isolated", async ({ page }) => {
  const { inline } = await pendingRun(page);
  await page.reload();
  await inline.getByRole("button", { name: "审批详情", exact: true }).click();
  // The panel divider is a separator (not a slider); its range is the live
  // three-column budget, and End may make the rail yield (design §5.0).
  const width = page.getByRole("separator", { name: /面板宽度/ });
  const widthNow = async () => Number(await width.getAttribute("aria-valuenow"));
  const widthMax = async () => Number(await width.getAttribute("aria-valuemax"));

  // A fresh browser context has no stored preference, so the default applies.
  expect(await widthNow()).toBe(WORK_PANEL_DEFAULT_WIDTH);

  await width.focus();
  await width.press("ArrowLeft");
  expect(await widthNow()).toBe(WORK_PANEL_DEFAULT_WIDTH + 16);
  await width.press("Shift+ArrowLeft");
  expect(await widthNow()).toBe(WORK_PANEL_DEFAULT_WIDTH + 48);
  await width.press("Home");
  expect(await widthNow()).toBe(WORK_PANEL_MIN_WIDTH);
  await width.press("ArrowRight");
  expect(await widthNow()).toBe(WORK_PANEL_MIN_WIDTH);
  await width.press("ArrowLeft");
  expect(await widthNow()).toBe(WORK_PANEL_MIN_WIDTH + 16);
  // The width is a global UI preference (design §3 ownership table), so it
  // survives a session switch; only the panel *selection* is session-isolated.
  await page.getByRole("button", { name: "Session B", exact: true }).click();
  // Select-then-paint: the pane follows the click one committed frame later.
  await expect(page.getByRole("heading", { name: "Session B" })).toBeVisible();
  await expect(page.getByRole("region", { name: "审批详情", exact: true })).toHaveCount(0);
  // Convergence, not a single read: a session switch re-mounts the panel, which
  // reports the default width until the stored preference is applied again. Polling
  // still fails if the preference is genuinely lost, but no longer depends on which
  // frame the read lands in.
  await expect.poll(widthNow).toBe(WORK_PANEL_MIN_WIDTH + 16);
  await page.getByRole("button", { name: /^Session A(?:,|$)/ }).click();
  await expect.poll(widthNow).toBe(WORK_PANEL_MIN_WIDTH + 16);

  // Widening to the live maximum may make the rail yield; the conversation keeps
  // its floor either way (design §5.0 priority order).
  await width.press("End");
  expect(await widthNow()).toBeLessThanOrEqual(await widthMax());
  const mainBox = await page.locator(".product-main").boundingBox();
  expect(mainBox?.width ?? 0).toBeGreaterThanOrEqual(MAIN_PANE_MIN_WIDTH);
  await expect(page.getByRole("region", { name: "审批详情", exact: true }).getByRole("button", { name: "批准", exact: true })).toBeVisible();
});

test("narrow panel traps focus and Escape returns to the actual details trigger", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 375, height: 812 });
  const { inline } = await pendingRun(page);
  const trigger = inline.getByRole("button", { name: "审批详情", exact: true });
  await trigger.click();
  const dialog = page.getByRole("dialog", { name: "详情", exact: true });
  await expect(dialog).toBeVisible();
  const close = dialog.getByRole("button", { name: "关闭详情", exact: true });
  await expect(close).toBeFocused();
  await close.press("Shift+Tab");
  await expect(dialog.getByRole("button", { name: "拒绝", exact: true })).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(close).toBeFocused();
  await page.screenshot({ path: testInfo.outputPath("approval-narrow.png") });
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(trigger).toBeFocused();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test("late approval response cannot clear another session's pending submission", async ({ page }) => {
  const { inline } = await pendingRun(page);
  let releaseA!: () => void;
  let releaseB!: () => void;
  const gateA = new Promise<void>((resolve) => { releaseA = resolve; });
  const gateB = new Promise<void>((resolve) => { releaseB = resolve; });
  let posts = 0;
  await page.route(/\/api\/jobs\/[^/]+\/approvals\/[^/]+$/, async (route) => {
    posts++;
    await (posts === 1 ? gateA : gateB);
    await route.fulfill({ status: 409, json: { error: "stale" } });
  });
  await inline.getByRole("button", { name: "批准", exact: true }).click();
  await page.getByRole("button", { name: "Session B", exact: true }).click();
  // Select-then-paint: the pane follows the click one committed frame later.
  await expect(page.getByRole("heading", { name: "Session B" })).toBeVisible();
  await page.getByRole("textbox", { name: /输入消息/ }).fill("B note");
  await page.getByRole("button", { name: "发送", exact: true }).click();
  await inline.getByRole("button", { name: "拒绝", exact: true }).click();
  releaseA();
  await expect(inline.getByRole("button", { name: "批准", exact: true })).toBeDisabled();
  await expect(page.getByText(/Approval outcome not confirmed/)).toHaveCount(0);
  expect(posts).toBe(2);
  releaseB();
  await expect(inline.getByRole("button", { name: "批准", exact: true })).toBeEnabled();
});

test("opening a file does not discard pending pagination or leave loading stuck", async ({ page }) => {
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  let started!: () => void;
  const pending = new Promise<void>((resolve) => { started = resolve; });
  const { inline, workspace } = await pendingRun(page);
  await page.route(/\/api\/product\/workspaces\/[^/]+\/files(?:\?|$)/, async (route) => {
    const more = new URL(route.request().url()).searchParams.has("cursor");
    if (more) { started(); await gate; }
    await route.fulfill({ json: {
      workspace_id: workspace.id, truncated: true,
      entries: [{ path: more ? "second.txt" : "first.txt", kind: "file", size: 12 }],
      next_cursor: more ? "page-3" : "page-2", scan_limit_reached: false,
    } });
  });
  await page.route(/\/api\/product\/workspaces\/[^/]+\/files\/content\?/, async (route) => {
    await route.fulfill({ json: { path: "first.txt", mime: "text/plain", size: 12,
      text: "selected file content", truncated: false, preview_allowed: false } });
  });
  await inline.getByRole("button", { name: "审批详情", exact: true }).click();
  // The panel is a dynamic tab strip, so the browser tab is opened from the
  // launcher instead of always being present.
  const panel = page.getByRole("tabpanel");
  await page.getByRole("button", { name: "打开一个页签", exact: true }).click();
  await panel.getByRole("button", { name: "文件", exact: true }).click();
  const files = page.getByRole("region", { name: "文件", exact: true });
  await expect(files.getByRole("button", { name: "first.txt 12 B", exact: true })).toBeVisible();
  await files.getByRole("button", { name: "加载更多", exact: true }).click();
  await pending;
  await files.getByRole("button", { name: "first.txt 12 B", exact: true }).click();
  await expect(files.getByText("selected file content", { exact: true })).toBeVisible();
  release();
  await expect(files.getByRole("button", { name: "second.txt 12 B", exact: true })).toBeVisible();
  await expect(files.getByRole("button", { name: "加载更多", exact: true })).toBeEnabled();
  await expect(files.getByText("selected file content", { exact: true })).toBeVisible();
});
