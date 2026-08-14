import { expect, test, type Page } from "@playwright/test";

import {
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
} from "./product-api-mock";

test("all nine settings sections expose a usable surface", async ({ page }) => {
  await installMockProductApi(page);
  await page.goto("/settings/general");

  await expect(page.getByRole("button", { name: "Light", exact: true })).toBeVisible();

  await page.getByRole("button", { name: "Providers & Models", exact: true }).click();
  await expect(page.getByRole("button", { name: "Save profile" })).toBeVisible();

  await page.getByRole("button", { name: "Tools & Approvals", exact: true }).click();
  await expect(page.getByRole("button", { name: "Save default" })).toBeVisible();

  await page.getByRole("button", { name: "Workspace / Paths", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Known workspaces" })).toBeVisible();

  await page.getByRole("button", { name: "Memory", exact: true }).click();
  await expect(
    page.getByText("Select a workspace to inspect its durable memory."),
  ).toBeVisible();

  await page.getByRole("button", { name: "Sessions", exact: true }).click();
  await expect(page.getByRole("heading", { name: "No sessions" })).toBeVisible();

  await page.getByRole("button", { name: "Keyboard shortcuts", exact: true }).click();
  await expect(page.getByText("Focus message composer")).toBeVisible();

  await page.getByRole("button", { name: "Advanced / Developer", exact: true }).click();
  await expect(page.getByRole("button", { name: /Benchmark runner/u })).toBeVisible();

  await page.getByRole("button", { name: "About / Runtime", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Resume health" })).toBeVisible();
  await expect(page.getByText("0.1.0", { exact: true })).toBeVisible();

  await expect(page.getByText(/intentionally a placeholder/iu)).toHaveCount(0);
  await expect(page.getByText(/Scaffolded for M1/iu)).toHaveCount(0);
});

test("provider profiles support durable create and update", async ({ page }) => {
  const api = await installMockProductApi(page);
  const initialCatalogRevision = api.providerCatalogRevision;
  await page.goto("/settings/providers");

  await page.getByLabel("Label").fill("Relay A");
  await page.getByLabel("API base").fill("https://relay-a.test/v1");
  await page.getByLabel("API key env name").fill("RELAY_A_KEY");
  await page.getByLabel("Default model").fill("relay/model-a");
  await page.getByRole("button", { name: "Save profile" }).click();

  const originalRow = page.locator(".profile-row").filter({ hasText: "Relay A" });
  await expect(originalRow).toBeVisible();
  const createdCatalogRevision = api.providerCatalogRevision;
  expect(createdCatalogRevision).not.toBe(initialCatalogRevision);
  expect(api.providerProfileMutationRequests[0]).toEqual({
    method: "POST",
    expectedRevision: initialCatalogRevision,
  });
  await originalRow.getByRole("button", { name: "Edit" }).click();
  await expect(page.getByRole("heading", { name: "Edit profile" })).toBeVisible();

  await page.getByLabel("Label").fill("Relay Updated");
  await page.getByLabel("API base").fill("https://relay-updated.test/v1");
  await page.getByLabel("Default model").fill("relay/model-b");
  await page.getByRole("button", { name: "Update profile" }).click();

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
  const darkThemeButton = page.getByRole("button", { name: "Dark", exact: true });
  await expect(darkThemeButton).toBeVisible();

  api.preferences = {
    ...api.preferences,
    revision: 1,
    theme: "light",
  };
  await darkThemeButton.click();

  await expect.poll(() => api.preferenceUpdateRequests).toBe(1);
  await expect(page.locator(".shell-alert")).toContainText(
    "preferences revision does not match",
  );
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  expect(api.preferences.revision).toBe(1);
});

test("quick model control persists the session-scoped next-run model", async ({ page }) => {
  const { api, workspace, session } = await installQuickModelFixture(page);

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  const trigger = page.getByRole("button", { name: "Change session model settings" });
  await trigger.click();
  const dialog = page.getByRole("dialog", { name: "Session model settings" });
  await expect(dialog).toContainText("Changes apply from the next run");
  await dialog.getByLabel("Session model").fill("relay/model-v2");
  await dialog.getByLabel("Session max steps").fill("19");
  await dialog.getByRole("button", { name: "Save session model" }).click();

  await expect(page.getByText("Session model updated.")).toBeVisible();
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
  await page.getByRole("button", { name: "Change session model settings" }).click();
  const dialog = page.getByRole("dialog", { name: "Session model settings" });
  const modelInput = dialog.getByLabel("Session model");
  await modelInput.fill("relay/model-not-saved");
  await dialog.getByRole("button", { name: "Save session model" }).click();

  await expect(dialog.getByRole("alert")).toContainText(
    "session model settings were not changed",
  );
  await expect.poll(() => modelInput.inputValue()).toBe("relay/model-a");
  expect(api.sessionModelConfigs[session.id]?.model).toBe("relay/model-a");
  await expect(page.locator(".shell-alert")).toContainText(
    "session model settings unavailable",
  );
});

test("quick model control recovers a session-config CAS conflict to server truth", async ({
  page,
}) => {
  const { api, workspace, session } = await installQuickModelFixture(page);

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  const trigger = page.getByRole("button", { name: "Change session model settings" });
  await expect(trigger).toBeVisible();
  await trigger.click();
  const dialog = page.getByRole("dialog", { name: "Session model settings" });
  const modelInput = dialog.getByLabel("Session model");
  Object.assign(api.sessionModelConfigs[session.id]!, {
    model: "relay/server-confirmed",
    revision: 2,
  });
  await modelInput.fill("relay/stale-write");
  await dialog.getByRole("button", { name: "Save session model" }).click();

  await expect(dialog.getByRole("alert")).toContainText(
    "session model settings were not changed",
  );
  await expect.poll(() => modelInput.inputValue()).toBe("relay/server-confirmed");
  await expect(page.locator(".shell-alert")).toContainText("revision does not match");
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
  await page.getByRole("button", { name: "Never", exact: true }).click();
  await expect
    .poll(() => api.preferences.default_approval_policy)
    .toBe("never");
  expect(api.preferences.revision).toBe(1);
  expect(api.preferences.provider_selection).toBeUndefined();

  await page.getByRole("button", { name: "Back to chat" }).click();
  await expect(page).toHaveURL(`/w/${workspace.id}/s/${session.id}`);
  await page.getByRole("textbox", { name: "Message" }).fill("Default policy turn");
  await page.getByRole("button", { name: "Send" }).click();
  await expect.poll(() => api.jobs).toHaveLength(1);
  await expect(page.getByRole("textbox", { name: "Message" })).toBeEnabled();
  expect(api.jobs[0]).not.toHaveProperty("approval");

  await page.getByRole("button", { name: "Open settings" }).click();
  await page.getByRole("button", { name: "Tools & Approvals", exact: true }).click();
  await page.getByLabel("Default maximum steps for new sessions").fill("17");
  await page.getByRole("button", { name: "Save default" }).click();
  await expect
    .poll(() => selectedMaxSteps(api.preferences))
    .toBe(17);

  await page.getByRole("button", { name: "Back to chat" }).click();
  await page.getByRole("textbox", { name: "Message" }).fill("Explicit limit turn");
  await page.getByRole("button", { name: "Send" }).click();
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
  const mcpSettings = page.getByLabel("MCP servers");
  const mcpForm = mcpSettings.locator("form");
  await mcpForm.getByLabel("Server name").fill("mock_server");
  await mcpForm.getByLabel("Command").fill("python");
  await mcpForm
    .getByLabel("Arguments (one per line)")
    .fill("tests/fixtures/mcp_mock_server.py\n--verbose");
  await mcpForm.getByLabel("Environment names (one per line)").fill("MCP_TOKEN");
  await mcpForm.getByLabel("Connection timeout (ms)").fill("2400");
  await expect(mcpForm.getByLabel("Required at startup")).toBeChecked();
  await mcpForm.getByRole("button", { name: "Add server" }).click();

  await expect(mcpForm.getByRole("alert")).toContainText(
    "MCP config is locked",
  );
  await expect(mcpForm.getByLabel("Server name")).toHaveValue("mock_server");
  await expect(mcpForm.getByLabel("Command")).toHaveValue("python");
  await expect(mcpForm.getByLabel("Arguments (one per line)")).toHaveValue(
    "tests/fixtures/mcp_mock_server.py\n--verbose",
  );
  await expect(
    mcpForm.getByLabel("Environment names (one per line)"),
  ).toHaveValue("MCP_TOKEN");

  await mcpForm.getByRole("button", { name: "Add server" }).click();
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

  await serverRow.getByRole("button", { name: "Edit" }).click();
  await mcpForm.getByLabel("Enabled").uncheck();
  await mcpForm.getByLabel("Required at startup").uncheck();
  await mcpForm.getByLabel("Connection timeout (ms)").fill("4500");
  await mcpForm.getByRole("button", { name: "Save changes" }).click();
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

  await serverRow.getByRole("button", { name: "Test" }).click();
  await expect(serverRow.getByRole("alert")).toContainText(
    "compatible MCP tool catalog",
  );
  await serverRow.getByRole("button", { name: "Test" }).click();
  await expect(serverRow).toContainText("2 tools");
  await expect(
    serverRow.getByText("mcp__mock_server__echo_remote", { exact: true }),
  ).toBeVisible();
  await expect(serverRow.getByRole("alert")).toHaveCount(0);

  await page.getByRole("button", { name: "Workspace / Paths", exact: true }).click();
  await page
    .locator(".profile-row")
    .filter({ hasText: workspaceB.canonical_root })
    .getByRole("button", { name: "Open" })
    .click();
  await expect.poll(() => api.preferences.active_workspace_id).toBe(workspaceB.id);
  await page.getByRole("button", { name: "Open settings" }).click();
  await page.getByRole("button", { name: "Tools & Approvals", exact: true }).click();
  await expect(page.getByText("No MCP servers in this workspace.")).toBeVisible();
  expect(api.mcpServers[workspaceA.id]).toHaveLength(1);
  expect(api.mcpServers[workspaceB.id] ?? []).toHaveLength(0);

  await page.getByRole("button", { name: "Workspace / Paths", exact: true }).click();
  await page
    .locator(".profile-row")
    .filter({ hasText: workspaceA.canonical_root })
    .getByRole("button", { name: "Open" })
    .click();
  await expect.poll(() => api.preferences.active_workspace_id).toBe(workspaceA.id);
  await page.getByRole("button", { name: "Open settings" }).click();
  await page.getByRole("button", { name: "Tools & Approvals", exact: true }).click();
  serverRow = page
    .getByLabel("MCP servers")
    .locator(".profile-row")
    .filter({ hasText: "mock_server" });
  await serverRow.getByRole("button", { name: "Remove" }).click();
  await expect(serverRow).toContainText(
    "Remove mock_server from this workspace?",
  );
  await serverRow.getByRole("button", { name: "Confirm remove" }).click();
  await expect(page.getByText("No MCP servers in this workspace.")).toBeVisible();
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
  await sessionRow.getByRole("button", { name: "Rename" }).click();
  await page.getByLabel("Session name").fill("Renamed session");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect.poll(() => api.sessions.find((item) => item.id === sessionB.id)?.title).toBe(
    "Renamed session",
  );

  sessionRow = page.locator(".profile-row").filter({ hasText: "Renamed session" });
  const [download] = await Promise.all([
    page.waitForEvent("download"),
    sessionRow.getByRole("button", { name: "Evidence export" }).click(),
  ]);
  expect(download.suggestedFilename()).toBe(
    "rove-session-session-b-evidence.json",
  );

  await sessionRow.getByRole("button", { name: "Delete" }).click();
  await sessionRow.getByRole("button", { name: "Confirm delete" }).click();
  await expect.poll(() => api.sessions.some((item) => item.id === sessionB.id)).toBe(false);

  await page.getByRole("button", { name: "Workspace / Paths", exact: true }).click();
  const workspaceRow = page.locator(".profile-row").filter({ hasText: workspace.display_name });
  await workspaceRow.getByRole("button", { name: "Pin", exact: true }).click();
  await expect.poll(() => api.workspaces[0]?.pinned).toBe(true);

  await workspaceRow.getByRole("button", { name: "Remove", exact: true }).click();
  await workspaceRow.getByRole("button", { name: "Confirm remove" }).click();
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
  await page.getByLabel("Search").fill("does-not-match");
  await page.getByRole("button", { name: "Search", exact: true }).click();
  await expect(
    page.getByText("No durable memory topics match these filters."),
  ).toBeVisible();
  await page.getByRole("button", { name: "Clear", exact: true }).click();

  await page.getByRole("button", { name: "Open", exact: true }).click();
  await expect(page.getByLabel("Memory topic content")).toContainText(
    "Run pnpm test before handoff.",
  );
  await expect(page.getByLabel("Memory topic metadata")).toContainText(
    "durable",
  );
  await page.getByRole("button", { name: "Edit topic" }).click();
  let editor = page.getByRole("region", { name: "Edit durable topic" });
  await editor.getByLabel("Title").fill("Updated Project Conventions");
  await editor
    .getByLabel("Content")
    .fill("Run pnpm test and browser acceptance before handoff.");
  await editor.getByRole("button", { name: "Save changes" }).click();
  await expect(editor.getByRole("alert")).toContainText(
    "memory topic changed concurrently",
  );
  await expect(editor.getByLabel("Title")).toHaveValue(
    "Updated Project Conventions",
  );
  await expect(editor.getByLabel("Content")).toHaveValue(
    "Run pnpm test and browser acceptance before handoff.",
  );
  await editor.getByRole("button", { name: "Save changes" }).click();
  await expect(
    page
      .getByLabel("Durable memory topics")
      .getByText("Updated Project Conventions", { exact: true }),
  ).toBeVisible();
  expect(api.memoryTopics["project-conventions"]?.topic.source).toBe(
    "product_settings",
  );

  await page.getByRole("button", { name: "New topic" }).click();
  editor = page.getByRole("region", { name: "New durable topic" });
  await editor.getByLabel("Slug").fill("session-scoped-reference");
  await editor.getByLabel("Title").fill("Session Scoped Reference");
  await editor.getByLabel("Type").selectOption("reference");
  await editor.getByLabel("Durable scope").selectOption("session");
  await editor.getByLabel("Confidence").fill("0.85");
  await editor.getByLabel("Description").fill("A durable session-scoped fact");
  await editor.getByLabel("Content").fill("This remains in durable memory.");
  await editor.getByRole("button", { name: "Create topic" }).click();
  await expect(
    page.locator(".profile-row").filter({ hasText: "Session Scoped Reference" }),
  ).toContainText("Durable · reference · session scope");
  expect(api.memoryTopics["session-scoped-reference"]?.topic).toMatchObject({
    layer: "durable",
    scope: "session",
    source: "product_settings",
  });

  await page.getByRole("button", { name: "Delete topic" }).click();
  await page.getByRole("button", { name: "Confirm delete" }).click();
  await expect(
    page.getByText("Session Scoped Reference", { exact: true }),
  ).toHaveCount(0);
  expect(api.memoryTopics["session-scoped-reference"]).toBeUndefined();
  expect(api.memoryTopics["project-conventions"]).toBeDefined();
  expect(api.memoryWorkspaceRequests).not.toHaveLength(0);
  expect(new Set(api.memoryWorkspaceRequests)).toEqual(new Set([workspace.id]));

  await page.getByRole("button", { name: "About / Runtime", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Resume health" })).toBeVisible();
  await expect(page.getByText("1", { exact: true }).first()).toBeVisible();

  await page.getByRole("button", { name: "Back to chat" }).click();
  await expect(page.getByRole("textbox", { name: "Message" })).toBeEnabled();
  await page.keyboard.press("/");
  await expect(page.getByRole("textbox", { name: "Message" })).toBeFocused();
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());

  await page.keyboard.press("Control+.");
  await expect(page.getByLabel("Run inspector")).toHaveAttribute(
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
    page.getByText("Select a workspace to inspect its durable memory."),
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
    await expect(page.getByRole("heading", { name: "Workspace / Paths" })).toBeVisible();
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

    await page.getByRole("button", { name: "Sessions", exact: true }).click();
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
