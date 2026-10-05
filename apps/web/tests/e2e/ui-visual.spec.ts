import { expect, test, type Page } from "@playwright/test";

import { installMockProductApi } from "./product-api-mock";

const WORKSPACE_ROOT = "D:/tmp/rove-shell-demo";
const SHOT_DIR = "test-results/ui-visual";

async function shot(page: Page, name: string) {
  await page.screenshot({ path: `${SHOT_DIR}/${name}.png` });
}

async function openWorkspace(page: Page) {
  await page.getByLabel("绝对路径").fill(WORKSPACE_ROOT);
  await page.getByRole("button", { name: "打开工作区", exact: true }).click();
  await expect(page).toHaveURL(/\/w\/workspace-1\/s\/session-1$/u);
  await expect(page.getByRole("textbox", { name: /输入消息/ })).toBeVisible();
}

function composer(page: Page) {
  return page.getByRole("textbox", { name: /输入消息/ });
}

test("visual: completed run, folded activity, tool detail, actions, usage ring", async ({
  page,
}) => {
  await installMockProductApi(page);
  await page.goto("/");
  await openWorkspace(page);
  await composer(page).fill("Summarize the runtime state");
  await page.getByRole("button", { name: "发送" }).click();
  await expect(
    page.getByLabel("Conversation").getByText("Runtime summary complete"),
  ).toBeVisible();
  // The default mock run has no tool calls: the transcript shows two bubbles.
  await shot(page, "01-shell-completed");

  const bubble = page.locator('article[data-role="assistant"]').first();
  await bubble.hover();
  await shot(page, "02-message-actions");

  await page.locator(".chat-composer").screenshot({ path: `${SHOT_DIR}/03-composer.png` });
});

test("visual: slash menu and file mention menu", async ({ page }) => {
  await installMockProductApi(page);
  await page.goto("/");
  await openWorkspace(page);

  await composer(page).pressSequentially("/", { delay: 40 });
  await expect(page.locator(".composer-menu")).toBeVisible();
  await shot(page, "07-slash-menu");

  await composer(page).fill("");
  await composer(page).pressSequentially("@src", { delay: 40 });
  // The files endpoint is not mocked: the menu must show its empty note.
  await expect(page.locator(".composer-menu")).toBeVisible();
  await page.waitForTimeout(300);
  await shot(page, "08-file-menu");

  // IME-safe close: Escape.
  await page.keyboard.press("Escape");
  await expect(page.locator(".composer-menu")).toHaveCount(0);
});

test("visual: paste chip", async ({ page }) => {
  await installMockProductApi(page);
  await page.goto("/");
  await openWorkspace(page);

  const bigText = Array.from({ length: 40 }, (_, index) => `line-${index} of pasted log content`).join("\n");
  await page.evaluate((text) => {
    const target = document.querySelector("textarea") as HTMLTextAreaElement;
    const clipboard = new DataTransfer();
    clipboard.setData("text/plain", text);
    const event = new ClipboardEvent("paste", { clipboardData: clipboard, bubbles: true, cancelable: true });
    target.dispatchEvent(event);
  }, bigText);
  await expect(page.locator(".paste-chip")).toBeVisible();
  await shot(page, "09-paste-chip");

  // Expand the chip: full text is viewable read-only.
  await page.locator(".paste-chip summary").click();
  await shot(page, "10-paste-chip-expanded");
});

test("visual: approval card outside the fold", async ({ page }) => {
  await installMockProductApi(page, { mode: "approval" });
  await page.goto("/");
  await openWorkspace(page);
  await composer(page).fill("Write a note");
  await page.getByRole("button", { name: "发送" }).click();
  await expect(page.getByLabel("审批")).toBeVisible();
  await shot(page, "11-approval");

  // A second send while the run is busy queues a successor message with
  // edit/move actions.
  await composer(page).fill("queued follow-up");
  await page.getByRole("button", { name: "发送" }).click();
  const queued = page.locator(".queued-message").first();
  await expect(queued).toBeVisible();
  await queued.hover();
  await page.waitForTimeout(150);
  await shot(page, "12-queued-actions");

  // Approve: the tool runs to completion and the activity folds itself.
  await page.getByRole("button", { name: "批准" }).click();
  await expect(
    page.getByLabel("Conversation").getByText("Approved write completed"),
  ).toBeVisible();
  await expect(page.getByText(/工具调用 ×/).first()).toBeVisible();
  await shot(page, "13-activity-folded");

  await page.getByText(/工具调用 ×/).first().click();
  await expect(page.locator(".tool-card").first()).toBeVisible();
  await shot(page, "14-activity-expanded");

  await page.locator(".tool-card__head").first().click();
  await shot(page, "15-tool-detail");
});

test("visual: sidebar pin, rename editor", async ({ page }) => {
  await installMockProductApi(page);
  await page.goto("/");
  await openWorkspace(page);

  const sessionRow = page.locator(".session-item").first();
  await sessionRow.hover();
  await page.waitForTimeout(150);
  await shot(page, "13-sidebar-hover");

  await page.getByRole("button", { name: "置顶会话" }).first().click();
  await page.waitForTimeout(150);
  await shot(page, "14-sidebar-pinned");

  await page.locator(".session-item").first().dblclick();
  await expect(page.locator(".session-item--editing input")).toBeVisible();
  await shot(page, "15-sidebar-rename");
  await page.keyboard.press("Escape");
});

test("visual: settings keyboard overrides, font scale, breathe", async ({ page }) => {
  await installMockProductApi(page);
  // Deep link straight into the keyboard section: the breathe highlight is
  // active for ~3.6s after landing.
  await page.goto("/settings/keyboard");
  await expect(page.getByRole("heading", { name: "键盘快捷键" })).toBeVisible();
  await shot(page, "16-settings-breathe");
  await page.waitForTimeout(4_200);
  await shot(page, "17-settings-breathe-after");

  await page.getByRole("button", { name: "修改按键" }).first().click();
  await expect(page.locator("kbd[data-recoring], kbd[data-recording]")).toBeVisible();
  await shot(page, "18-keyboard-recording");
  await page.keyboard.press("Escape");

  // Give focus-composer a custom binding, then verify the settings row shows it.
  await page.getByRole("button", { name: "修改按键" }).first().click();
  await page.keyboard.press("b");
  await page.waitForTimeout(150);
  await shot(page, "19-keyboard-bound");

  await page.locator(".settings-nav").getByRole("button", { name: "通用" }).click();
  await shot(page, "20-settings-general-font");
  await page.getByRole("button", { name: "放大字号" }).click();
  await page.getByRole("button", { name: "放大字号" }).click();
  await page.getByRole("button", { name: "放大字号" }).click();
  await shot(page, "21-font-scaled-settings");

  // Back to the chat: the reading surface should render at the larger scale.
  await page.getByRole("button", { name: "返回会话", exact: true }).click();
  await page.waitForTimeout(400);
  await shot(page, "22-font-scaled-chat");
});
