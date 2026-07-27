import { expect, test, type Page } from "@playwright/test";

import { installMockProductApi } from "./product-api-mock";

const WORKSPACE_ROOT = "D:/tmp/rove-shell-demo";

test("empty -> open workspace -> run -> complete on live shell mock", async ({
  page,
}) => {
  const api = await installMockProductApi(page);

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Open a workspace to start" })).toBeVisible();

  await openWorkspace(page);
  await page.getByRole("textbox", { name: "Message" }).fill("Summarize the runtime state");
  await page.getByRole("button", { name: "Send" }).click();

  const conversation = page.getByLabel("Conversation");
  await expect(conversation.getByText("Summarize the runtime state")).toBeVisible();
  await expect(conversation.getByText("Runtime summary complete")).toBeVisible();
  await expect(page.getByLabel("Run inspector").getByText("Run completed", { exact: true })).toBeVisible();
  expect(api.jobs).toHaveLength(1);
  expect(api.jobs[0]).toMatchObject({
    product_session_id: "session-1",
    workspace: { kind: "folder", root: WORKSPACE_ROOT },
  });
  expect(api.jobs[0]).not.toHaveProperty("resume");
});

test("inline approval works in product shell", async ({ page }) => {
  await installMockProductApi(page, { mode: "approval" });

  await page.goto("/");
  await openWorkspace(page);
  await page.getByRole("textbox", { name: "Message" }).fill("Write a note");
  await page.getByRole("button", { name: "Send" }).click();

  const approval = page.getByLabel("Pending approval");
  await expect(approval).toBeVisible();
  await expect(approval).toBeFocused();
  await expect(approval.getByRole("button", { name: "Approve" })).not.toBeFocused();
  await expect(
    approval.getByText("destructive tool requires explicit approval"),
  ).toBeVisible();

  await Promise.all([
    page.waitForResponse(
      (response) =>
        response.url().includes("/approvals/") && response.status() === 200,
    ),
    page.getByRole("button", { name: "Approve" }).click(),
  ]);

  await expect(page.getByLabel("Conversation").getByText("Approved write completed")).toBeVisible();
});

test("theme toggle flips data-theme on the document", async ({ page }) => {
  await installMockProductApi(page);
  await page.goto("/");

  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await page.getByRole("button", { name: "Switch to dark theme" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.getByRole("button", { name: "Switch to light theme" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
});

test("inspector shows empty then completed states during a run", async ({ page }) => {
  await installMockProductApi(page);
  await page.goto("/");
  await openWorkspace(page);

  const inspector = page.getByLabel("Run inspector");
  await expect(inspector.getByText("No active run")).toBeVisible();
  await expect(inspector.getByText(/Plan, tools, and approvals/)).toBeVisible();

  await page.getByRole("textbox", { name: "Message" }).fill("Summarize the runtime state");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByLabel("Conversation").getByText("Runtime summary complete")).toBeVisible();
  await expect(inspector.getByText("Run completed", { exact: true })).toBeVisible();
});

test("benchmark lives under Settings Advanced, not primary nav", async ({ page }) => {
  await installMockProductApi(page);
  await page.route(/\/api\/bench\/.*/u, async (route) => {
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({ suites: [], runs: [] }),
    });
  });

  await page.goto("/");
  await expect(page.getByRole("button", { name: "Benchmarks" })).toHaveCount(0);
  await page.getByRole("button", { name: "Open settings" }).click();
  await page.getByRole("button", { name: "Advanced / Developer" }).click();
  await expect(page).toHaveURL(/\/settings\/advanced$/u);
  await expect(page.getByRole("heading", { name: "Advanced / Developer" })).toBeVisible();
  await page.getByRole("button", { name: /Benchmark runner/ }).click();
  await expect(page.getByLabel("Benchmark runner")).toBeVisible();
  await expect(page.getByRole("heading", { name: "Benchmark Runner" })).toBeVisible();
});

test("settings providers can test and list models without raw keys", async ({
  page,
}) => {
  let sawModels = false;
  let sawTest = false;
  await installMockProductApi(page);
  await page.route("/api/providers/models", async (route) => {
    const body = route.request().postDataJSON() as {
      provider?: { api_key_env?: string; api_base?: string };
    };
    sawModels =
      body.provider?.api_base === "https://gateway.test/v1" &&
      body.provider.api_key_env === "GATEWAY_API_KEY" &&
      !JSON.stringify(body).includes("sk-");
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        provider: "gateway.test",
        provider_type: "openai",
        wire_protocol: "openai_chat",
        api_base: "https://gateway.test/v1",
        key_env: "GATEWAY_API_KEY",
        key_present: true,
        models: ["relay/model-a", "relay/model-b"],
        models_count: 2,
      }),
    });
  });
  await page.route("/api/providers/test", async (route) => {
    const body = route.request().postDataJSON() as {
      provider?: { api_key_env?: string; api_base?: string };
    };
    sawTest =
      body.provider?.api_base === "https://gateway.test/v1" &&
      body.provider.api_key_env === "GATEWAY_API_KEY";
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        status: "pass",
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

  await page.goto("/");
  await page.getByRole("button", { name: "Open settings" }).click();
  await expect(page).toHaveURL(/\/settings\/providers$/u);
  await expect(page.getByRole("heading", { name: "Providers & Models" })).toBeVisible();

  await page.getByLabel("API base").fill("https://gateway.test/v1");
  await page.getByLabel("API key env name").fill("GATEWAY_API_KEY");
  await page.getByLabel("Default model").fill("relay/model-a");
  await page.getByRole("button", { name: "List models" }).click();
  await expect(page.getByText(/Models \(2\):/)).toBeVisible();
  await page.getByRole("button", { name: "Test" }).click();
  await expect(page.getByText(/Test: pass/)).toBeVisible();

  await expect.poll(() => sawModels).toBe(true);
  await expect.poll(() => sawTest).toBe(true);
});

async function openWorkspace(page: Page) {
  await page.getByLabel("Absolute path").fill(WORKSPACE_ROOT);
  await page.getByRole("button", { name: "Open workspace", exact: true }).click();
  await expect(page).toHaveURL(/\/w\/workspace-1\/s\/session-1$/u);
  await expect(page.getByRole("textbox", { name: "Message" })).toBeVisible();
}
