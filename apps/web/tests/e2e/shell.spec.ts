import { expect, test, type Page } from "@playwright/test";

import { installMockProductApi } from "./product-api-mock";

const WORKSPACE_ROOT = "D:/tmp/rove-shell-demo";

test("empty -> open workspace -> run -> complete on live shell mock", async ({
  page,
}) => {
  const api = await installMockProductApi(page);

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "打开工作区以开始" })).toBeVisible();

  await openWorkspace(page);
  await page.getByRole("textbox", { name: /输入消息/ }).fill("Summarize the runtime state");
  await page.getByRole("button", { name: "发送" }).click();

  const conversation = page.getByLabel("Conversation");
  await expect(conversation.getByText("Summarize the runtime state")).toBeVisible();
  await expect(conversation.getByText("Runtime summary complete")).toBeVisible();
  await expect(page.getByLabel("详情").getByText("已完成", { exact: true })).toBeVisible();
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
  await page.getByRole("textbox", { name: /输入消息/ }).fill("Write a note");
  await page.getByRole("button", { name: "发送" }).click();

  const approval = page.getByLabel("审批");
  await expect(approval).toBeVisible();
  await expect(approval).toBeFocused();
  await expect(approval.getByRole("button", { name: "批准" })).not.toBeFocused();
  await expect(
    approval.getByText("destructive tool requires explicit approval"),
  ).toBeVisible();

  await Promise.all([
    page.waitForResponse(
      (response) =>
        response.url().includes("/approvals/") && response.status() === 200,
    ),
    page.getByRole("button", { name: "批准" }).click(),
  ]);

  await expect(page.getByLabel("Conversation").getByText("Approved write completed")).toBeVisible();
});

test("theme toggle flips data-theme on the document", async ({ page }) => {
  await installMockProductApi(page);
  await page.goto("/");

  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await page.getByRole("button", { name: "切换到深色主题" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.getByRole("button", { name: "切换到浅色主题" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
});

test("inspector shows empty then completed states during a run", async ({ page }) => {
  await installMockProductApi(page);
  await page.goto("/");
  await openWorkspace(page);

  const inspector = page.getByLabel("详情");
  await expect(inspector.getByText("暂无进行中的运行")).toBeVisible();
  await expect(inspector.getByText(/发送一条消息/)).toBeVisible();

  await page.getByRole("textbox", { name: /输入消息/ }).fill("Summarize the runtime state");
  await page.getByRole("button", { name: "发送" }).click();
  await expect(page.getByLabel("Conversation").getByText("Runtime summary complete")).toBeVisible();
  await expect(inspector.getByText("已完成", { exact: true })).toBeVisible();
});

test("benchmark runner is not exposed in product Settings", async ({ page }) => {
  await installMockProductApi(page);
  await page.goto("/");
  await page.getByLabel("设置", { exact: true }).click();
  await expect(page.locator(".settings-nav").getByRole("button", { name: "高级", exact: true })).toHaveCount(0);
  await expect(page).toHaveURL(/\/settings\/providers$/u);
  await expect(page.getByRole("button", { name: /Benchmark runner/ })).toHaveCount(0);
  await expect(page.getByLabel("Benchmark runner")).toHaveCount(0);
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
  await page.getByLabel("设置", { exact: true }).click();
  await expect(page).toHaveURL(/\/settings\/providers$/u);
  await expect(page.getByRole("heading", { name: "模型服务" })).toBeVisible();

  await page.getByLabel("API 地址").fill("https://gateway.test/v1");
  await page.locator("summary").filter({ hasText: "高级选项" }).click();
  await page.getByLabel("密钥环境变量名").fill("GATEWAY_API_KEY");
  await page.getByLabel("默认模型").fill("relay/model-a");
  await page.getByRole("button", { name: "列出模型" }).click();
  await expect(page.getByText("可用模型（2）", { exact: false })).toBeVisible();
  await page.getByRole("button", { name: "测试连接" }).click();
  await expect(page.getByText(/已连接/)).toBeVisible();

  await expect.poll(() => sawModels).toBe(true);
  await expect.poll(() => sawTest).toBe(true);
});

async function openWorkspace(page: Page) {
  await page.getByLabel("绝对路径").fill(WORKSPACE_ROOT);
  await page.getByRole("button", { name: "打开工作区", exact: true }).click();
  await expect(page).toHaveURL(/\/w\/workspace-1\/s\/session-1$/u);
  await expect(page.getByRole("textbox", { name: /输入消息/ })).toBeVisible();
}
