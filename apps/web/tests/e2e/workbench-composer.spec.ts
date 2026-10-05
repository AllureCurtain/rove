import { expect, test, type Page } from "@playwright/test";

import { installMockProductApi } from "./product-api-mock";

async function openComposer(
  page: Page,
  options?: Parameters<typeof installMockProductApi>[1],
) {
  const api = await installMockProductApi(page, options);
  await page.goto("/");
  await page.getByLabel("绝对路径").fill("D:/tmp/rove-composer-demo");
  await page.getByRole("button", { name: "打开工作区", exact: true }).click();
  const composer = page.getByRole("textbox", { name: /输入消息/ });
  await expect(composer).toBeEnabled();
  return { api, composer };
}

for (const shortcut of ["Control+Enter", "Meta+Enter"]) {
  test(`${shortcut} sends the trimmed draft and clears it on success`, async ({ page }) => {
    const { api, composer } = await openComposer(page);
    await composer.fill("  keyboard message  ");
    await composer.press(shortcut);
    await expect(page.getByLabel("Conversation").getByText("keyboard message", { exact: true })).toBeVisible();
    await expect(composer).toHaveValue("");
    expect(api.jobs).toHaveLength(1);
  });
}

test("Enter and Shift+Enter insert newlines without sending", async ({ page }) => {
  const { api, composer } = await openComposer(page);
  await composer.fill("first");
  await composer.press("Enter");
  await composer.press("Shift+Enter");
  await expect(composer).toHaveValue("first\n\n");
  expect(api.jobs).toHaveLength(0);
  await composer.fill("   ");
  await composer.press("Control+Enter");
  await expect(page.getByRole("button", { name: "发送", exact: true })).toBeDisabled();
  expect(api.jobs).toHaveLength(0);
});

test("IME confirmation never submits a draft", async ({ page }) => {
  const { api, composer } = await openComposer(page);
  await composer.fill("中文输入");
  for (const event of [
    { key: "Enter", ctrlKey: true, isComposing: true },
    { key: "Enter", metaKey: true, keyCode: 229 },
  ]) {
    await composer.dispatchEvent("keydown", event);
    await expect(composer).toHaveValue("中文输入");
    await expect(composer).toBeEnabled();
    expect(api.jobs).toHaveLength(0);
  }
  await composer.press("Control+Enter");
  await expect(composer).toHaveValue("");
  expect(api.jobs).toHaveLength(1);
});

test("switching sessions preserves isolated in-memory drafts", async ({ page }) => {
  const { composer } = await openComposer(page);
  const firstUrl = page.url();
  await composer.fill("  first session draft  ");
  await page.locator(".product-sidebar").getByRole("button", { name: "新会话", exact: true }).click();
  await expect(page).not.toHaveURL(firstUrl);
  const secondUrl = page.url();
  await expect(composer).toBeEnabled();
  await expect(composer).toHaveValue("");
  await composer.fill("second session draft");
  await page.goBack();
  await expect(page).toHaveURL(firstUrl);
  await expect(composer).toHaveValue("  first session draft  ");
  await page.goForward();
  await expect(page).toHaveURL(secondUrl);
  await expect(composer).toHaveValue("second session draft");
});

test("rejected send retains the exact draft and permits explicit retry", async ({ page }) => {
  const { api, composer } = await openComposer(page);
  let reject = true;
  let attempts = 0;
  await page.route("**/api/product/sessions/*/messages", async (route) => {
    if (route.request().method() !== "POST") return route.fallback();
    attempts += 1;
    if (reject) {
      return route.fulfill({ status: 400, contentType: "application/json",
        body: JSON.stringify({ code: "invalid_request", error: "Draft rejection fixture" }) });
    }
    return route.fallback();
  });
  const draft = "  keep this draft\nexactly  ";
  await composer.fill(draft);
  await composer.press("Control+Enter");
  await expect(page.locator("#composer-error")).toContainText("Draft rejection fixture");
  await expect(composer).toBeEnabled();
  await expect(composer).toHaveValue(draft);
  expect(attempts).toBe(1);
  expect(api.jobs).toHaveLength(0);
  reject = false;
  await page.getByRole("button", { name: "发送", exact: true }).click();
  await expect(composer).toHaveValue("");
  expect(attempts).toBe(2);
  expect(api.jobs).toHaveLength(1);
});

test("pending send cannot duplicate or clear another session draft", async ({ page }) => {
  const { api, composer } = await openComposer(page);
  const originalUrl = page.url();
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  let attempts = 0;
  await page.route("**/api/product/sessions/*/messages", async (route) => {
    if (route.request().method() !== "POST") return route.fallback();
    attempts += 1;
    await gate;
    return route.fallback();
  });
  try {
    await composer.fill("pending original");
    // Two events in the same browser task exercise the synchronous lock,
    // before React has a chance to commit a disabled textarea.
    await composer.evaluate((element) => {
      for (let i = 0; i < 2; i += 1) {
        element.dispatchEvent(new KeyboardEvent("keydown", {
          key: "Enter", ctrlKey: true, bubbles: true, cancelable: true,
        }));
      }
    });
    await expect.poll(() => attempts).toBe(1);
    await expect(composer).toBeDisabled();
    await page.locator(".product-sidebar").getByRole("button", { name: "新会话", exact: true }).click();
    await expect(page).not.toHaveURL(originalUrl);
    await expect(composer).toBeEnabled();
    await composer.fill("new session text");
    release();
    await expect.poll(() => api.jobs.length).toBe(1);
    await expect(composer).toHaveValue("new session text");
    expect(attempts).toBe(1);
  } finally {
    release();
  }
});

test("repeated shortcut keydown never sends", async ({ page }) => {
  const { api, composer } = await openComposer(page);
  await composer.fill("not from a held key");
  const prevented = await composer.evaluate((element) => {
    const event = new KeyboardEvent("keydown", {
      key: "Enter", ctrlKey: true, repeat: true, bubbles: true, cancelable: true,
    });
    element.dispatchEvent(event);
    return event.defaultPrevented;
  });
  expect(prevented).toBe(true);
  await expect(composer).toHaveValue("not from a held key");
  expect(api.jobs).toHaveLength(0);
});

test("stop remains available while a run is busy and cancels it", async ({ page }) => {
  await openComposer(page, { mode: "approval" });
  const composer = page.getByRole("textbox", { name: /输入消息/ });
  await composer.fill("needs approval");
  await composer.press("Control+Enter");
  await expect(page.getByRole("button", { name: "停止" })).toBeVisible();
  // Submission briefly disables editing; an active run does not, because users
  // can prepare/send a successor message while Stop remains independently usable.
  await expect(composer).toBeEnabled();
  await composer.fill("keep this draft after stopping");
  const stop = page.getByRole("button", { name: "停止", exact: true });
  await expect(stop).toBeEnabled();
  await stop.click();
  await expect(page.getByRole("button", { name: "停止" })).toBeHidden();
  await expect(composer).toBeEnabled();
  await expect(composer).toHaveValue("keep this draft after stopping");
});
