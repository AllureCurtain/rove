import { expect, test, type Locator, type Page } from "@playwright/test";

import { installMockProductApi } from "./product-api-mock";

/**
 * F6: a stop puts the last send back the way it was written, including the paste
 * chips that the outgoing message had folded into its text.
 */

const PASTED = Array.from(
  { length: 40 },
  (_, index) => `line-${index} of pasted log content`,
).join("\n");

async function pasteIntoComposer(composer: Locator, text: string) {
  await composer.evaluate((element, pasted) => {
    const clipboard = new DataTransfer();
    clipboard.setData("text/plain", pasted);
    element.dispatchEvent(
      new ClipboardEvent("paste", {
        clipboardData: clipboard,
        bubbles: true,
        cancelable: true,
      }),
    );
  }, text);
}

async function openWaitingRun(page: Page) {
  const api = await installMockProductApi(page, { mode: "waiting_model" });
  await page.goto("/");
  await page.getByLabel("绝对路径").fill("D:/tmp/rove-smart-stop");
  await page.getByRole("button", { name: "打开工作区", exact: true }).click();
  const composer = page.getByRole("textbox", { name: /输入消息/ });
  await expect(composer).toBeEnabled();
  return { api, composer, stop: page.getByRole("button", { name: "停止", exact: true }) };
}

test("a stopped send comes back with its pasted chip", async ({ page }) => {
  const { composer, stop } = await openWaitingRun(page);

  await composer.fill("explain this log");
  await pasteIntoComposer(composer, PASTED);
  await expect(page.locator(".paste-chip")).toHaveCount(1);

  await composer.press("Control+Enter");
  await expect(stop).toBeVisible();
  // The message that went out carries the paste folded into its text; the chip
  // itself is gone, which is what makes the restore below meaningful.
  await expect(composer).toHaveValue("");
  await expect(page.locator(".paste-chip")).toHaveCount(0);

  await stop.click();

  await expect(composer).toHaveValue("explain this log");
  const chip = page.locator(".paste-chip");
  await expect(chip).toHaveCount(1);
  await expect(chip.locator(".paste-chip__preview")).toContainText(
    "line-0 of pasted log content",
  );
  // Both surfaces say where the draft came from.
  await expect(page.locator(".chat-composer__paste-notice")).toContainText(
    "已恢复 1 条发送前的粘贴内容",
  );
  await expect(page.locator(".shell-alert")).toContainText(
    "发送前的草稿（含粘贴内容）已完整放回输入框",
  );

  // The restored draft is a real one: sending it again takes the chip with it.
  await composer.press("Control+Enter");
  await expect(composer).toHaveValue("");
  await expect(page.locator(".paste-chip")).toHaveCount(0);
  await expect(stop).toBeVisible();
});

test("without a snapshot the stop falls back to the folded text", async ({ page }) => {
  const { composer, stop } = await openWaitingRun(page);

  await composer.fill("explain this log");
  await pasteIntoComposer(composer, PASTED);
  await composer.press("Control+Enter");
  await expect(stop).toBeVisible();

  // Drafts are memory-only, so a reload is exactly the case that has no
  // snapshot: the stop can only put the folded message text back.
  await page.reload();
  const reloaded = page.getByRole("textbox", { name: /输入消息/ });
  await expect(reloaded).toBeEnabled();
  const reloadedStop = page.getByRole("button", { name: "停止", exact: true });
  await expect(reloadedStop).toBeVisible();
  await reloadedStop.click();

  await expect(reloaded).toHaveValue(/explain this log/);
  await expect(reloaded).toHaveValue(/\[pasted content: \d+ characters\]/);
  await expect(page.locator(".paste-chip")).toHaveCount(0);
  await expect(page.locator(".chat-composer__paste-notice")).toHaveCount(0);
  await expect(page.locator(".shell-alert")).toContainText(
    "刚才的消息已放回输入框",
  );
});
