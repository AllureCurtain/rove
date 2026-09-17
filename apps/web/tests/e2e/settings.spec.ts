import { expect, test, type Page } from "@playwright/test";

import {
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
} from "./product-api-mock";

test("all eight settings sections expose a usable surface", async ({ page }) => {
  await installMockProductApi(page);
  await page.goto("/settings/general");

  await expect(page.getByRole("button", { name: "浅色", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "中文", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "冷色精修", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "暖米色", exact: true }).click();
  await expect(page.locator(".product-app-frame")).toHaveAttribute("data-skin", "warm");
  await page.getByRole("button", { name: "冷色精修", exact: true }).click();
  await expect(page.locator(".product-app-frame")).toHaveAttribute("data-skin", "cool");

  await page.getByRole("button", { name: "模型服务", exact: true }).click();
  await expect(page.getByRole("button", { name: "保存更改" })).toBeVisible();

  await page.getByRole("button", { name: "工具与审批", exact: true }).click();
  await expect(page.getByRole("button", { name: "保存默认值" })).toBeVisible();

  await page.getByRole("button", { name: "工作区路径", exact: true }).click();
  await expect(page.getByRole("heading", { name: "已知工作区" })).toBeVisible();

  await page.getByRole("button", { name: "记忆", exact: true }).click();
  await expect(
    page.getByText("请先选择工作区以查看其持久记忆。"),
  ).toBeVisible();

  await page.getByRole("button", { name: "会话", exact: true }).click();
  await expect(page.getByRole("heading", { name: "暂无会话" })).toBeVisible();

  await page.getByRole("button", { name: "键盘快捷键", exact: true }).click();
  await expect(page.getByText("聚焦输入框")).toBeVisible();

  await expect(page.locator(".settings-nav").getByRole("button", { name: "高级", exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: /Benchmark runner/u })).toHaveCount(0);

  await page.getByRole("button", { name: "关于", exact: true }).click();
  await expect(page.getByRole("heading", { name: "会话恢复" })).toBeVisible();
  await expect(page.getByText("0.1.0", { exact: true })).toBeVisible();

  await expect(page.getByText(/intentionally a placeholder/iu)).toHaveCount(0);
  await expect(page.getByText(/Scaffolded for M1/iu)).toHaveCount(0);
});

test("old advanced links open usable General settings without an empty section", async ({ page }) => {
  await installMockProductApi(page);
  await page.goto("/settings/advanced");
  await expect(page.getByRole("button", { name: "中文", exact: true })).toBeVisible();
  await expect(page.locator(".settings-nav").getByRole("button", { name: "通用", exact: true })).toHaveAttribute("aria-current", "page");
  await expect(page.locator(".settings-nav").getByRole("button", { name: "高级", exact: true })).toHaveCount(0);
});

test("provider profiles support durable create and update", async ({ page }) => {
  const api = await installMockProductApi(page);
  const initialCatalogRevision = api.providerCatalogRevision;
  await page.goto("/settings/providers");

  await page.getByLabel("名称").fill("Relay A");
  await page.getByLabel("API 地址").fill("https://relay-a.test/v1");
  await expect(page.getByLabel("密钥环境变量名")).not.toBeVisible();
  await page.locator("summary").filter({ hasText: "高级选项" }).click();
  await page.getByLabel("密钥环境变量名").fill("RELAY_A_KEY");
  await page.getByLabel("默认模型").fill("relay/model-a");
  await page.getByRole("button", { name: "保存更改" }).click();

  const originalRow = page.locator(".profile-row").filter({ hasText: "Relay A" });
  await expect(originalRow).toBeVisible();
  const createdCatalogRevision = api.providerCatalogRevision;
  expect(createdCatalogRevision).not.toBe(initialCatalogRevision);
  expect(api.providerProfileMutationRequests[0]).toEqual({
    method: "POST",
    expectedRevision: initialCatalogRevision,
  });
  await originalRow.getByRole("button", { name: "编辑" }).click();
  await expect(page.getByRole("heading", { name: "编辑服务" })).toBeVisible();

  await page.getByLabel("名称").fill("Relay Updated");
  await page.getByLabel("API 地址").fill("https://relay-updated.test/v1");
  await page.getByLabel("默认模型").fill("relay/model-b");
  await page.getByRole("button", { name: "保存更改" }).click();

  await expect(
    page.locator(".profile-row").filter({ hasText: "Relay Updated" }),
  ).toBeVisible();
  await expect.poll(() => api.providerProfiles[0]?.label).toBe("Relay Updated");
  expect(api.providerCatalogRevision).not.toBe(createdCatalogRevision);
  expect(api.providerProfileMutationRequests[1]).toMatchObject({
    method: "PUT",
    expectedRevision: createdCatalogRevision,
  });
  expect(api.providerProfiles[0]).toMatchObject({
    api_base: "https://relay-updated.test/v1",
    default_model: "relay/model-b",
  });
});

test("stale preference revisions recover to the server-confirmed snapshot", async ({ page }) => {
  const api = await installMockProductApi(page);
  await page.goto("/settings/general");
  const darkThemeButton = page.getByRole("button", { name: "深色", exact: true });
  await expect(darkThemeButton).toBeVisible();

  api.preferences = {
    ...api.preferences,
    revision: 1,
    theme: "light",
  };
  await darkThemeButton.click();

  await expect.poll(() => api.preferenceUpdateRequests).toBe(1);
  await expect(page.locator(".shell-alert")).toContainText(
    "设置操作未完成",
  );
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  expect(api.preferences.revision).toBe(1);
});

test("quick model control persists the session-scoped next-run model", async ({ page }) => {
  const { api, workspace, session } = await installQuickModelFixture(page);

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  const trigger = page.getByRole("button", { name: "更改会话模型设置" });
  await trigger.click();
  const dialog = page.getByRole("dialog", { name: "会话模型" });
  await expect(dialog).toContainText("更改将在本会话的下一轮生效。");
  await dialog.getByLabel("会话模型", { exact: true }).fill("relay/model-v2");
  await dialog.getByLabel("会话最大步数").fill("19");
  await dialog.getByRole("button", { name: "保存会话模型" }).click();

  await expect(page.getByText("会话模型已更新。")).toBeVisible();
  await expect(trigger).toBeFocused();
  await expect
    .poll(() => api.sessionModelConfigs[session.id]?.model)
    .toBe("relay/model-v2");
  expect(api.sessionModelConfigs[session.id]).toMatchObject({
    profile_id: "profile-relay",
    max_steps: 19,
    revision: 2,
  });
  expect(api.sessionModelConfigUpdateRequests).toBe(1);
});

test("quick model control rolls back after a real session-config failure", async ({ page }) => {
  const { api, workspace, session } = await installQuickModelFixture(page, {
    sessionModelConfigFailures: 1,
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await page.getByRole("button", { name: "更改会话模型设置" }).click();
  const dialog = page.getByRole("dialog", { name: "会话模型" });
  const modelInput = dialog.getByLabel("会话模型", { exact: true });
  await modelInput.fill("relay/model-not-saved");
  await dialog.getByRole("button", { name: "保存会话模型" }).click();

  await expect(dialog.getByRole("alert")).toContainText("会话模型设置未更改");
  await expect.poll(() => modelInput.inputValue()).toBe("relay/model-a");
  expect(api.sessionModelConfigs[session.id]?.model).toBe("relay/model-a");
  await expect(page.locator(".shell-alert")).toContainText(
    "设置操作未完成",
  );
});

test("quick model control recovers a session-config CAS conflict to server truth", async ({
  page,
}) => {
  const { api, workspace, session } = await installQuickModelFixture(page);

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  const trigger = page.getByRole("button", { name: "更改会话模型设置" });
  await expect(trigger).toBeVisible();
  await trigger.click();
  const dialog = page.getByRole("dialog", { name: "会话模型" });
  const modelInput = dialog.getByLabel("会话模型", { exact: true });
  Object.assign(api.sessionModelConfigs[session.id]!, {
    model: "relay/server-confirmed",
    revision: 2,
  });
  await modelInput.fill("relay/stale-write");
  await dialog.getByRole("button", { name: "保存会话模型" }).click();

  await expect(dialog.getByRole("alert")).toContainText("会话模型设置未更改");
  await expect.poll(() => modelInput.inputValue()).toBe("relay/server-confirmed");
  await expect(page.locator(".shell-alert")).toContainText("设置操作未完成");
  expect(api.sessionModelConfigs[session.id]?.model).toBe(
    "relay/server-confirmed",
  );
});

test("approval defaults and execution limits affect later job requests", async ({ page }) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });

  await page.goto("/settings/tools");
  await page.getByRole("button", { name: "从不", exact: true }).click();
  await expect
    .poll(() => api.preferences.default_approval_policy)
    .toBe("never");
  expect(api.preferences.revision).toBe(1);
  expect(api.preferences.provider_selection).toBeUndefined();

  await page.getByRole("button", { name: "返回会话" }).click();
  await expect(page).toHaveURL(`/w/${workspace.id}/s/${session.id}`);
  await page.getByRole("textbox", { name: /输入消息/ }).fill("Default policy turn");
  await page.getByRole("button", { name: "发送" }).click();
  await expect.poll(() => api.jobs).toHaveLength(1);
  await expect(page.getByRole("textbox", { name: /输入消息/ })).toBeEnabled();
  expect(api.jobs[0]).not.toHaveProperty("approval");

  await page.getByLabel("设置", { exact: true }).click();
  await page.getByRole("button", { name: "工具与审批", exact: true }).click();
  await page.getByLabel("新会话默认最大步数").fill("17");
  await page.getByRole("button", { name: "保存默认值" }).click();
  await expect
    .poll(() => selectedMaxSteps(api.preferences))
    .toBe(17);

  await page.getByRole("button", { name: "返回会话" }).click();
  await page.getByRole("textbox", { name: /输入消息/ }).fill("Explicit limit turn");
  await page.getByRole("button", { name: "发送" }).click();
  await expect.poll(() => api.jobs).toHaveLength(2);
  expect(api.jobs[1]).not.toHaveProperty("approval");
  expect(api.jobs[1]).not.toHaveProperty("max_steps");
  expect(api.jobEffectiveConfigs[1]).toMatchObject({
    approval: "never",
    max_steps: 8,
  });
});

test("MCP settings preserve failed drafts, recover probes, and isolate workspaces", async ({
  page,
}) => {
  const workspaceA = createMockWorkspace(
    "workspace-mcp-a",
    "D:/tmp/rove-mcp-a",
  );
  const workspaceB = createMockWorkspace(
    "workspace-mcp-b",
    "D:/tmp/rove-mcp-b",
  );
  const sessionA = createMockSession(
    "session-mcp-a",
    workspaceA.id,
    "MCP A",
  );
  const sessionB = createMockSession(
    "session-mcp-b",
    workspaceB.id,
    "MCP B",
  );
  const api = await installMockProductApi(page, {
    workspaces: [workspaceA, workspaceB],
    sessions: [sessionA, sessionB],
    activeWorkspaceId: workspaceA.id,
    activeSessionId: sessionA.id,
    mcpMutationFailures: 1,
    mcpProbeFailures: {
      [`${workspaceA.id}:mock_server`]: [
        {
          status: 502,
          code: "product_mcp_protocol_mismatch",
          error: "the MCP server returned an incompatible protocol response",
        },
      ],
    },
  });

  await page.goto("/settings/tools");
  const mcpSettings = page.getByLabel("MCP 服务");
  const mcpForm = mcpSettings.locator("form");
  await mcpForm.getByLabel("服务名称").fill("mock_server");
  await mcpForm.getByLabel("命令").fill("python");
  await mcpForm
    .getByLabel("参数（每行一个）")
    .fill("tests/fixtures/mcp_mock_server.py\n--verbose");
  await mcpForm.getByLabel("环境变量名（每行一个）").fill("MCP_TOKEN");
  await mcpForm.getByLabel("连接超时（毫秒）").fill("2400");
  await expect(mcpForm.getByLabel("启动时必需")).toBeChecked();
  await mcpForm.getByRole("button", { name: "添加服务" }).click();

  await expect(mcpForm.getByRole("alert")).toContainText(
    "MCP config is locked",
  );
  await expect(mcpForm.getByLabel("服务名称")).toHaveValue("mock_server");
  await expect(mcpForm.getByLabel("命令")).toHaveValue("python");
  await expect(mcpForm.getByLabel("参数（每行一个）")).toHaveValue(
    "tests/fixtures/mcp_mock_server.py\n--verbose",
  );
  await expect(
    mcpForm.getByLabel("环境变量名（每行一个）"),
  ).toHaveValue("MCP_TOKEN");

  await mcpForm.getByRole("button", { name: "添加服务" }).click();
  let serverRow = mcpSettings
    .locator(".profile-row")
    .filter({ hasText: "mock_server" });
  await expect(serverRow).toContainText("Enabled");
  await expect(serverRow).toContainText("Required");
  await expect(serverRow).toContainText("health: unknown");
  await expect(serverRow).toContainText("2400 ms");
  await expect.poll(() => api.mcpMutationRequests).toBe(2);
  expect(api.mcpServers[workspaceA.id]).toHaveLength(1);
  expect(api.mcpServers[workspaceB.id] ?? []).toHaveLength(0);
  for (const rawBody of api.mcpRequestBodies) {
    expect(JSON.parse(rawBody)).not.toHaveProperty("env");
  }
  expect(JSON.parse(api.mcpRequestBodies.at(-1)!)).toMatchObject({
    env_names: ["MCP_TOKEN"],
    required: true,
  });

  await serverRow.getByRole("button", { name: "编辑" }).click();
  await mcpForm.getByLabel("已启用").uncheck();
  await mcpForm.getByLabel("启动时必需").uncheck();
  await mcpForm.getByLabel("连接超时（毫秒）").fill("4500");
  await mcpForm.getByRole("button", { name: "保存更改" }).click();
  serverRow = mcpSettings
    .locator(".profile-row")
    .filter({ hasText: "mock_server" });
  await expect(serverRow).toContainText("Disabled");
  await expect(serverRow).toContainText("Optional");
  await expect(serverRow).toContainText("health: disabled");
  await expect(serverRow).toContainText("4500 ms");
  expect(api.mcpServers[workspaceA.id]?.[0]).toMatchObject({
    enabled: false,
    required: false,
    request_timeout_ms: 4500,
  });
  expect(JSON.parse(api.mcpRequestBodies.at(-1)!)).not.toHaveProperty("env");

  await serverRow.getByRole("button", { name: "测试连接" }).click();
  await expect(serverRow.getByRole("alert")).toContainText(
    "compatible MCP tool catalog",
  );
  await serverRow.getByRole("button", { name: "测试连接" }).click();
  await expect(serverRow).toContainText("2 tools");
  await expect(
    serverRow.getByText("mcp__mock_server__echo_remote", { exact: true }),
  ).toBeVisible();
  await expect(serverRow.getByRole("alert")).toHaveCount(0);

  await page.getByRole("button", { name: "工作区路径", exact: true }).click();
  await page
    .locator(".profile-row")
    .filter({ hasText: workspaceB.canonical_root })
    .getByRole("button", { name: "打开工作区" })
    .click();
  await expect.poll(() => api.preferences.active_workspace_id).toBe(workspaceB.id);
  await page.getByLabel("设置", { exact: true }).click();
  await page.getByRole("button", { name: "工具与审批", exact: true }).click();
  await expect(page.getByText("该工作区暂无 MCP 服务。")).toBeVisible();
  expect(api.mcpServers[workspaceA.id]).toHaveLength(1);
  expect(api.mcpServers[workspaceB.id] ?? []).toHaveLength(0);

  await page.getByRole("button", { name: "工作区路径", exact: true }).click();
  await page
    .locator(".profile-row")
    .filter({ hasText: workspaceA.canonical_root })
    .getByRole("button", { name: "打开工作区" })
    .click();
  await expect.poll(() => api.preferences.active_workspace_id).toBe(workspaceA.id);
  await page.getByLabel("设置", { exact: true }).click();
  await page.getByRole("button", { name: "工具与审批", exact: true }).click();
  serverRow = page
    .getByLabel("MCP 服务")
    .locator(".profile-row")
    .filter({ hasText: "mock_server" });
  await serverRow.getByRole("button", { name: "移除" }).click();
  await expect(serverRow).toContainText(
    "从该工作区移除 mock_server？",
  );
  await serverRow.getByRole("button", { name: "移除" }).first().click();
  await expect(page.getByText("该工作区暂无 MCP 服务。")).toBeVisible();
  expect(api.mcpServers[workspaceA.id]).toEqual([]);
});

test("workspace and session settings mutate the durable catalog", async ({ page }) => {
  const workspace = createMockWorkspace();
  const sessionA = createMockSession("session-a", workspace.id, "Session A");
  const sessionB = createMockSession("session-b", workspace.id, "Session B");
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [sessionA, sessionB],
    activeWorkspaceId: workspace.id,
    activeSessionId: sessionA.id,
  });

  await page.goto("/settings/sessions");
  let sessionRow = page.locator(".profile-row").filter({ hasText: "Session B" });
  await sessionRow.getByRole("button", { name: "编辑" }).click();
  await page.getByLabel("会话名称").fill("Renamed session");
  await page.getByRole("button", { name: "保存", exact: true }).click();
  await expect.poll(() => api.sessions.find((item) => item.id === sessionB.id)?.title).toBe(
    "Renamed session",
  );

  sessionRow = page.locator(".profile-row").filter({ hasText: "Renamed session" });
  const [download] = await Promise.all([
    page.waitForEvent("download"),
    sessionRow.getByRole("button", { name: "导出对话（脱敏）" }).click(),
  ]);
  expect(download.suggestedFilename()).toBe(
    "rove-session-session-b-evidence.json",
  );

  await sessionRow.getByRole("button", { name: "删除" }).first().click();
  await sessionRow.getByRole("button", { name: "删除" }).first().click();
  await expect.poll(() => api.sessions.some((item) => item.id === sessionB.id)).toBe(false);

  await page.getByRole("button", { name: "工作区路径", exact: true }).click();
  const workspaceRow = page.locator(".profile-row").filter({ hasText: workspace.display_name });
  await workspaceRow.getByRole("button", { name: "固定", exact: true }).click();
  await expect.poll(() => api.workspaces[0]?.pinned).toBe(true);

  await workspaceRow.getByRole("button", { name: "移除", exact: true }).first().click();
  await expect(workspaceRow).toContainText("从目录中移除该工作区");
  await workspaceRow.getByRole("button", { name: "移除", exact: true }).first().click();
  await expect.poll(() => api.workspaces).toHaveLength(0);
  await expect(page).toHaveURL("/");
});

test("memory management, runtime health, and critical shortcuts are live", async ({ page }) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
    memoryMutationFailures: 1,
    memoryTopics: {
      "project-conventions": {
        topic: {
          slug: "project-conventions",
          title: "Project Conventions",
          layer: "durable",
          memory_type: "project",
          scope: "project",
          source: "llm_tool",
          confidence: 0.9,
          created_at: "2026-07-26T00:00:00Z",
          updated_at: "2026-07-27T00:00:00Z",
          description: "Repository checks",
          metadata_truncated: false,
        },
        content: "Run pnpm test before handoff.",
        truncated: false,
      },
    },
  });

  await page.goto("/settings/memory");
  await page.getByLabel("搜索").fill("does-not-match");
  await page.getByRole("button", { name: "搜索", exact: true }).click();
  await expect(
    page.getByText("没有符合筛选条件的持久记忆主题。"),
  ).toBeVisible();
  await page.getByRole("button", { name: "清除", exact: true }).click();

  await page.getByRole("button", { name: "打开", exact: true }).click();
  await expect(page.getByLabel("记忆主题内容")).toContainText(
    "Run pnpm test before handoff.",
  );
  await expect(page.getByLabel("主题信息")).toContainText("持久记忆");
  await expect(page.getByRole("button", { name: "编辑主题", exact: true })).toBeEnabled();
  await page.getByRole("button", { name: "编辑主题" }).click();
  let editor = page.getByRole("region", { name: "编辑持久主题" });
  await editor.getByLabel("标题").fill("Updated Project Conventions");
  await editor
    .getByLabel("内容")
    .fill("Run pnpm test and browser acceptance before handoff.");
  await editor.getByRole("button", { name: "保存更改" }).click();
  await expect(editor.getByRole("alert")).toContainText(
    "主题未保存，可能已被更改",
  );
  await expect(editor.getByLabel("标题")).toHaveValue(
    "Updated Project Conventions",
  );
  await expect(editor.getByLabel("内容")).toHaveValue(
    "Run pnpm test and browser acceptance before handoff.",
  );
  await editor.getByRole("button", { name: "保存更改" }).click();
  await expect(
    page
      .getByLabel("持久记忆主题")
      .getByText("Updated Project Conventions", { exact: true }),
  ).toBeVisible();
  expect(api.memoryTopics["project-conventions"]?.topic.source).toBe(
    "product_settings",
  );

  await page.getByRole("button", { name: "新建持久主题" }).click();
  editor = page.getByRole("region", { name: "新建持久主题" });
  await editor.getByLabel("标识").fill("session-scoped-reference");
  await editor.getByLabel("标题").fill("Session Scoped Reference");
  await editor.getByLabel("类型").selectOption("reference");
  await editor.getByLabel("作用域").selectOption("session");
  await editor.getByLabel("置信度").fill("0.85");
  await editor.getByLabel("说明").fill("A durable session-scoped fact");
  await editor.getByLabel("内容").fill("This remains in durable memory.");
  await editor.getByRole("button", { name: "创建主题" }).click();
  await expect(
    page.locator(".profile-row").filter({ hasText: "Session Scoped Reference" }),
  ).toContainText("参考 · 会话");
  expect(api.memoryTopics["session-scoped-reference"]?.topic).toMatchObject({
    layer: "durable",
    scope: "session",
    source: "product_settings",
  });

  await page.getByRole("button", { name: "删除主题" }).click();
  await page.getByRole("button", { name: "确认删除" }).click();
  await expect(
    page.getByText("Session Scoped Reference", { exact: true }),
  ).toHaveCount(0);
  expect(api.memoryTopics["session-scoped-reference"]).toBeUndefined();
  expect(api.memoryTopics["project-conventions"]).toBeDefined();
  expect(api.memoryWorkspaceRequests).not.toHaveLength(0);
  expect(new Set(api.memoryWorkspaceRequests)).toEqual(new Set([workspace.id]));

  await page.getByRole("button", { name: "关于", exact: true }).click();
  await expect(page.getByRole("heading", { name: "会话恢复" })).toBeVisible();
  await expect(page.getByText("1", { exact: true }).first()).toBeVisible();

  await page.getByRole("button", { name: "返回会话" }).click();
  await expect(page.getByRole("textbox", { name: /输入消息/ })).toBeEnabled();
  await page.keyboard.press("/");
  await expect(page.getByRole("textbox", { name: /输入消息/ })).toBeFocused();
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());

  await page.keyboard.press("Control+.");
  await expect(page.locator(".product-inspector").first()).toHaveAttribute(
    "data-collapsed",
    "true",
  );

  await page.keyboard.press("Control+Shift+Enter");
  await expect(page).toHaveURL(`/w/${workspace.id}/s/session-2`);
  await expect.poll(() => api.sessions).toHaveLength(2);

  await page.keyboard.press("Control+,");
  await expect(page).toHaveURL(/\/settings\/general$/u);
});

test("memory settings is explicit and does not query without a workspace", async ({
  page,
}) => {
  const api = await installMockProductApi(page);

  await page.goto("/settings/memory");

  await expect(
    page.getByText("请先选择工作区以查看其持久记忆。"),
  ).toBeVisible();
  expect(api.memoryWorkspaceRequests).toEqual([]);
});

test.describe("mobile settings", () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test("navigation and catalog content remain bounded", async ({ page }) => {
    const workspace = createMockWorkspace(
      "workspace-mobile",
      "D:/a/long/workspace/path/used/to/check/mobile/overflow",
    );
    const session = createMockSession(
      "session-mobile",
      workspace.id,
      "A long session title that must remain readable on mobile",
    );
    await installMockProductApi(page, {
      workspaces: [workspace],
      sessions: [session],
      activeWorkspaceId: workspace.id,
      activeSessionId: session.id,
    });

    await page.goto("/settings/workspace");
    await expect(page.getByRole("heading", { name: "工作区路径" })).toBeVisible();
    await expect(page.getByText(workspace.canonical_root)).toBeVisible();

    const layout = await page.evaluate(() => {
      const nav = document.querySelector<HTMLElement>(".settings-nav");
      const content = document.querySelector<HTMLElement>(".settings-content");
      const navRect = nav?.getBoundingClientRect();
      const contentRect = content?.getBoundingClientRect();
      return {
        documentWidth: document.documentElement.scrollWidth,
        viewportWidth: document.documentElement.clientWidth,
        separated:
          navRect !== undefined &&
          contentRect !== undefined &&
          navRect.bottom <= contentRect.top + 1,
      };
    });
    expect(layout.documentWidth).toBeLessThanOrEqual(layout.viewportWidth);
    expect(layout.separated).toBe(true);

    await page.getByRole("button", { name: "会话", exact: true }).click();
    await expect(page.getByText(session.title)).toBeVisible();
    await expect
      .poll(() =>
        page.evaluate(
          () => document.documentElement.scrollWidth <= document.documentElement.clientWidth,
        ),
      )
      .toBe(true);
  });
});

function selectedMaxSteps(preferences: Record<string, unknown>): number | undefined {
  const selection = preferences.provider_selection;
  if (!selection || typeof selection !== "object") {
    return undefined;
  }
  const value = (selection as Record<string, unknown>).max_steps;
  return typeof value === "number" ? value : undefined;
}

async function installQuickModelFixture(
  page: Page,
  options: { sessionModelConfigFailures?: number } = {},
) {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
    sessionModelConfigFailures: options.sessionModelConfigFailures,
    providerProfiles: [
      {
        id: "profile-relay",
        label: "Relay",
        provider_type: "openai",
        api_base: "https://relay.test/v1",
        api_key_env: "RELAY_KEY",
        default_model: "relay/model-a",
        created_at: "2026-07-28T00:00:00.000Z",
        updated_at: "2026-07-28T00:00:00.000Z",
      },
    ],
  });
  api.preferences = {
    ...api.preferences,
    provider_selection: {
      profile_id: "profile-relay",
      model: "relay/model-a",
      approval: "ask",
      max_steps: 24,
    },
  };
  Object.assign(api.sessionModelConfigs[session.id]!, {
    profile_id: "profile-relay",
    model: "relay/model-a",
    max_steps: 24,
  });
  return { api, workspace, session };
}
