import { expect, test } from "@playwright/test";
import { createMockSession, createMockWorkspace, installMockProductApi } from "./product-api-mock";

test("demo onboarding stays honest and provider errors identify the field", async ({ page }) => {
  await installMockProductApi(page, { providerSelection: { model: "fake" } });
  await page.goto("/");
  await expect(page.getByRole("note")).toContainText("离线演示");
  const checklist = page.getByRole("region", { name: "入门清单" });
  await expect(checklist.locator("li").first()).toHaveAttribute("data-done", "false");
  await checklist.getByRole("button", { name: "模型服务设置" }).click();
  await expect(page.getByRole("heading", { name: "新会话默认模型" })).toBeVisible();
  await page.getByLabel("API 地址", { exact: true }).fill("not-a-url");
  await page.getByRole("button", { name: "保存更改", exact: true }).click();
  await expect(page.locator("#provider-draft-error")).toContainText("http:// 或 https://");
  await expect(page.getByLabel("API 地址", { exact: true })).toBeFocused();
  await expect(page.getByLabel("API 地址", { exact: true })).toHaveAttribute("aria-invalid", "true");

  await page.getByLabel("API 地址", { exact: true }).fill("https://example.test/v1");
  await page.getByLabel("API 密钥", { exact: true }).fill("fixture-credential");
  await expect(page.getByRole("button", { name: "验证并保存" })).toBeEnabled();
  await expect(page.getByRole("button", { name: "测试连接", exact: true })).toBeDisabled();
  await expect(page.getByRole("button", { name: "列出模型", exact: true })).toBeDisabled();
  await page.getByLabel("API 密钥", { exact: true }).fill("");
});

for (const width of [760, 1280]) {
  test(`approval is actionable with an accurate wait at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 800 });
    const workspace = createMockWorkspace();
    const session = createMockSession();
    await installMockProductApi(page, {
      workspaces: [workspace], sessions: [session], activeWorkspaceId: workspace.id,
      activeSessionId: session.id, mode: "approval",
    });
    await page.goto(`/w/${workspace.id}/s/${session.id}`);
    await page.getByRole("textbox", { name: "输入消息", exact: true }).fill("Write a note");
    await page.getByRole("button", { name: "发送", exact: true }).click();
    const approval = page.getByRole("alert", { name: "审批" });
    await expect(approval).toBeVisible();
    await expect(page.locator(".transcript-runtime-status")).toContainText("等待你批准 write_file");
    await expect(page.locator(".transcript-runtime-status")).not.toContainText("正在执行");
    await approval.getByRole("button", { name: "批准", exact: true }).click();
    await expect(approval).toHaveCount(0);
  });
}
