import { expect, test } from "@playwright/test";

import {
  createMockSession,
  createMockSessionModelConfig,
  createMockWorkspace,
  installMockProductApi,
  type MockProviderProfile,
} from "./product-api-mock";

/**
 * F10 preferences (design §11): the per-model reasoning memory and the frontend
 * auto-title. Both are UI conveniences — neither changes a server contract.
 */

const RESPONSES_PROFILE: MockProviderProfile = {
  id: "profile-responses",
  label: "Responses",
  provider_type: "openai-responses",
  api_base: "https://api.example.test/v1",
  default_model: "gpt-5",
  created_at: "2026-09-26T00:00:00.000Z",
  updated_at: "2026-09-26T00:00:00.000Z",
  catalog_revision: "rev-1",
};

const SECOND_RESPONSES_PROFILE: MockProviderProfile = {
  ...RESPONSES_PROFILE,
  id: "profile-responses-mini",
  label: "Responses mini",
  default_model: "gpt-5-mini",
};

test("the reasoning effort follows the model it was chosen for", async ({
  page,
}) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    providerProfiles: [RESPONSES_PROFILE, SECOND_RESPONSES_PROFILE],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  api.sessionModelConfigs[session.id] = createMockSessionModelConfig(
    session.id,
    RESPONSES_PROFILE.id,
    RESPONSES_PROFILE.default_model!,
  );
  await page.goto(`/w/${workspace.id}/s/${session.id}`);

  const trigger = page.getByRole("button", { name: "更改会话模型设置" });
  const provider = page.getByLabel("会话模型服务");
  const model = page.getByRole("combobox", { name: "会话模型", exact: true });
  const reasoning = page.getByLabel("会话推理设置");

  // Model A: choose "high" and save it.
  await trigger.click();
  await expect(reasoning).toBeEnabled();
  await reasoning.selectOption("high");
  await page.getByRole("button", { name: "保存会话模型" }).click();
  await expect(page.locator(".quick-model__result")).toContainText("会话模型已更新");

  // Model B: a different model, a different effort.
  await trigger.click();
  await provider.selectOption(SECOND_RESPONSES_PROFILE.id);
  await expect(model).toHaveValue(SECOND_RESPONSES_PROFILE.default_model!);
  await expect(reasoning).toBeEnabled();
  await reasoning.selectOption("low");
  await page.getByRole("button", { name: "保存会话模型" }).click();
  await expect(page.locator(".quick-model__result")).toContainText("会话模型已更新");

  // Back to model A: the effort chosen for A comes back from the local memory,
  // even though the saved session config currently names B at "low".
  await trigger.click();
  await provider.selectOption(RESPONSES_PROFILE.id);
  await expect(model).toHaveValue(RESPONSES_PROFILE.default_model!);
  await expect(reasoning).toHaveValue("high");

  // And model B still remembers its own choice.
  await provider.selectOption(SECOND_RESPONSES_PROFILE.id);
  await expect(reasoning).toHaveValue("low");
});

test("the first message names a session the user never named", async ({
  page,
}) => {
  const workspace = createMockWorkspace();
  const fresh = createMockSession("session-fresh", workspace.id, "New session");
  const named = createMockSession("session-named", workspace.id, "Keep this title");
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [fresh, named],
    activeWorkspaceId: workspace.id,
    activeSessionId: fresh.id,
  });

  const message =
    "Measure the rail collapse before changing it, then write the numbers down";
  const truncated = `${Array.from(message).slice(0, 48).join("")}...`;

  await page.goto(`/w/${workspace.id}/s/${fresh.id}`);
  const composer = page.getByRole("textbox", { name: /输入消息/ });
  await expect(composer).toBeEnabled();
  await composer.fill(message);
  await composer.press("Control+Enter");

  // The server got the truncated first message as the title, and the shell shows
  // it without waiting for the next catalog read.
  await expect
    .poll(() => api.sessions.find((item) => item.id === fresh.id)?.title)
    .toBe(truncated);
  await expect(page.locator(".chat-pane__header h1")).toHaveText(truncated);

  // A session the user did name keeps its title: the guard is the placeholder
  // template, which is the approximation the design records.
  await page.goto(`/w/${workspace.id}/s/${named.id}`);
  const namedComposer = page.getByRole("textbox", { name: /输入消息/ });
  await expect(namedComposer).toBeEnabled();
  await namedComposer.fill("This must not rename anything");
  await namedComposer.press("Control+Enter");
  await expect(page.locator(".chat-pane__header h1")).toHaveText("Keep this title");
  expect(api.sessions.find((item) => item.id === named.id)?.title).toBe(
    "Keep this title",
  );
});
