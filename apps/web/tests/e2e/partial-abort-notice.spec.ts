import { expect, test, type Page } from "@playwright/test";

import { installMockProductApi } from "./product-api-mock";

/**
 * R2b: the composer notice for a stop that salvaged a partial answer.
 *
 * The marker is a fact about the stopped *run*, and the runtime writes it as the
 * terminal `llm_message` carrying the text produced so far. The notice therefore
 * has to be derived from that event arriving, not from a constant read while the
 * salvage window is still open — and it must not appear for a stop that kept
 * nothing, nor for old salvaged history a refresh brings back.
 */

async function openWaitingRun(page: Page, salvageOnCancel?: string) {
  const api = await installMockProductApi(page, {
    mode: "running_tool",
    ...(salvageOnCancel === undefined ? {} : { salvageOnCancel }),
  });
  await page.goto("/");
  await page.getByText("手动输入路径").click();
  await page.getByLabel("绝对路径").fill("D:/tmp/rove-partial-abort");
  await page.getByRole("button", { name: "打开工作区", exact: true }).click();
  const composer = page.getByRole("textbox", { name: /输入消息/ });
  await expect(composer).toBeEnabled();
  return {
    api,
    composer,
    stop: page.getByRole("button", { name: "停止", exact: true }),
  };
}

test("a stop that salvaged a partial answer says so in the composer", async ({
  page,
}) => {
  const { composer, stop } = await openWaitingRun(page, "The answer so far");

  await composer.fill("write me a long answer");
  await composer.press("Control+Enter");
  await expect(stop).toBeVisible();
  await stop.click();

  // The turn had already produced work, so the message stays and the notice
  // names what was kept.
  await expect(page.locator("p.shell-alert")).toContainText(
    "已保留在对话中",
  );
  await expect(page.getByText("The answer so far", { exact: false })).toBeVisible();
});

test("a stop that kept nothing shows no partial notice", async ({ page }) => {
  const { composer, stop } = await openWaitingRun(page);

  await composer.fill("write me a long answer");
  await composer.press("Control+Enter");
  await expect(stop).toBeVisible();
  await stop.click();

  // Nothing was salvaged, so the existing stop behaviour is untouched: no notice
  // may claim a partial that was never kept.
  await expect(stop).toBeHidden();
  await expect(page.getByText("已保留在对话中")).toHaveCount(0);
});

test("old salvaged history does not raise the notice on a refresh", async ({ page }) => {
  const { composer, stop } = await openWaitingRun(page, "The answer so far");

  await composer.fill("write me a long answer");
  await composer.press("Control+Enter");
  await expect(stop).toBeVisible();
  await stop.click();
  // Two surfaces say it, and they are different facts: the bubble carries the
  // marker for the message, the composer line announces what this stop kept.
  await expect(page.locator("p.shell-alert")).toContainText("已保留在对话中");
  await expect(page.locator(".message-abort-note")).toContainText("已保留在对话中");

  // A reload restores the same transcript — the salvaged partial included — but
  // this stop is over, so the notice must not come back with it. The bubble's own
  // note is history and stays.
  await page.reload();
  await expect(page.getByText("The answer so far", { exact: false })).toBeVisible();
  await expect(page.locator("p.shell-alert")).toHaveCount(0);
  await expect(page.locator(".message-abort-note")).toHaveCount(1);
});