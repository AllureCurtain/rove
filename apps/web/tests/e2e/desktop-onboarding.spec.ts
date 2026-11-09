import { expect, test } from "@playwright/test";
import { installMockProductApi } from "./product-api-mock";

test("a loopback-entry desktop sends credentials once and clears them before the response", async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperties(window, {
      __ROVE_API_URL__: { value: `${window.location.origin}/api` },
      __ROVE_TOKEN__: { value: "fixture-host-token" },
      __ROVE_CREDENTIAL_ENTRY__: { value: "loopback" },
    });
  });
  await installMockProductApi(page);
  let calls = 0;
  let release!: () => void;
  const pending = new Promise<void>((resolve) => { release = resolve; });
  await page.route("**/api/product/provider-onboarding", async (route) => {
    calls += 1;
    expect(route.request().headers().authorization).toBe("Bearer fixture-host-token");
    expect(route.request().postDataJSON().credential).toBe("fixture-provider-key");
    await pending;
    await route.fulfill({ status: 502, contentType: "application/json", body: JSON.stringify({
      code: "provider_authentication", error: "fixture-provider-key must not be echoed",
    }) });
  });
  await page.goto("/settings/providers");
  const key = page.getByLabel("API 密钥", { exact: true });
  await expect(key).toBeEnabled();
  await key.fill("fixture-provider-key");
  await page.getByRole("button", { name: "验证并保存", exact: true }).click();
  await expect(key).toHaveValue("");
  await expect(key).toBeDisabled();
  expect(calls).toBe(1);
  release();
  await expect(key).toBeEnabled();
  await expect(page.locator("#provider-draft-error")).toContainText("密钥验证失败");
  await expect(page.locator("body")).not.toContainText("fixture-provider-key");
  expect(calls).toBe(1);
  expect(await page.evaluate(() => JSON.stringify({ ...localStorage }))).not.toContain("fixture-provider-key");
});
