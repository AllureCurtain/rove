import { expect, test } from "@playwright/test";

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
  await expect(page.getByRole("button", { name: "Save limit" })).toBeVisible();

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
  await page.goto("/settings/providers");

  await page.getByLabel("Label").fill("Relay A");
  await page.getByLabel("API base").fill("https://relay-a.test/v1");
  await page.getByLabel("API key env name").fill("RELAY_A_KEY");
  await page.getByLabel("Default model").fill("relay/model-a");
  await page.getByRole("button", { name: "Save profile" }).click();

  const originalRow = page.locator(".profile-row").filter({ hasText: "Relay A" });
  await expect(originalRow).toBeVisible();
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
  expect(api.jobs[0]).not.toHaveProperty("approval");

  await page.getByRole("button", { name: "Open settings" }).click();
  await page.getByRole("button", { name: "Tools & Approvals", exact: true }).click();
  await page.getByLabel("Maximum steps per job").fill("17");
  await page.getByRole("button", { name: "Save limit" }).click();
  await expect
    .poll(() => selectedMaxSteps(api.preferences))
    .toBe(17);

  await page.getByRole("button", { name: "Back to chat" }).click();
  await page.getByRole("textbox", { name: "Message" }).fill("Explicit limit turn");
  await page.getByRole("button", { name: "Send" }).click();
  await expect.poll(() => api.jobs).toHaveLength(2);
  expect(api.jobs[1]).toMatchObject({ approval: "never", max_steps: 17 });
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
  const downloadPromise = page.waitForEvent("download");
  await sessionRow.getByRole("button", { name: "Export" }).click();
  const download = await downloadPromise;
  expect(download.suggestedFilename()).toBe("rove-session-session-b.json");

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
    memoryTopics: {
      "project-conventions": {
        topic: {
          slug: "project-conventions",
          title: "Project Conventions",
          memory_type: "project",
          scope: "project",
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
  await page.getByRole("button", { name: "Open", exact: true }).click();
  await expect(page.getByLabel("Memory topic content")).toContainText(
    "Run pnpm test before handoff.",
  );
  await page.getByRole("button", { name: "Delete topic" }).click();
  await page.getByRole("button", { name: "Confirm delete" }).click();
  await expect(page.getByText("No durable memory topics are available.")).toBeVisible();
  expect(api.memoryTopics["project-conventions"]).toBeUndefined();
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
