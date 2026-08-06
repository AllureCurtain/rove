import { expect, test, type Page } from "@playwright/test";

import { SERVER_CONFIRMED_THEME_CACHE_KEY } from "../../platform/server-theme-cache";
import { M1_BROWSER_STORAGE_KEYS } from "../../product/m1-storage-keys";
import {
  completedTranscript,
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
} from "./product-api-mock";

test.use({ viewport: { width: 390, height: 844 } });

test("mobile chat reflows, traps both production panels, and honors reduced motion", async ({
  page,
}, testInfo) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  const workspace = createMockWorkspace();
  const session = createMockSession();
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: {
      [session.id]: completedTranscript(
        workspace,
        session,
        "Mobile question",
        "Mobile answer",
      ),
    },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(page.getByText("Mobile answer", { exact: true })).toBeVisible();
  await expectNoHorizontalOverflow(page);

  const inspector = page.getByLabel("Run inspector");
  await expect(inspector).toHaveAttribute("data-collapsed", "true");
  const composerBox = await page.getByRole("textbox", { name: "Message" }).boundingBox();
  expect(composerBox?.width).toBeGreaterThan(240);

  const evidenceTrigger = page.getByRole("button", { name: "Open run evidence" });
  await evidenceTrigger.click();
  await expect(inspector).toHaveAttribute("role", "dialog");
  await expect(inspector).toHaveAttribute("aria-modal", "true");
  await expect(inspector.getByRole("button", { name: "Close run evidence" })).toBeFocused();
  await expect(page.locator(".product-main")).toHaveAttribute("inert", "");
  const expandedBox = await page.getByLabel("Run inspector").boundingBox();
  expect(expandedBox).not.toBeNull();
  expect(expandedBox!.x).toBeGreaterThanOrEqual(0);
  expect(expandedBox!.x + expandedBox!.width).toBeLessThanOrEqual(390);
  // The inspector now holds real evidence-export controls, so Tab advances
  // within the panel instead of wrapping to the close button. Assert the trap's
  // actual contract: focus stays inside, and it wraps at the boundaries.
  await page.keyboard.press("Tab");
  await expect(
    inspector.locator(":focus"),
    "Tab must keep focus inside the trapped inspector",
  ).toHaveCount(1);
  const closeButton = inspector.getByRole("button", { name: "Close run evidence" });
  await closeButton.focus();
  await page.keyboard.press("Shift+Tab");
  await expect(
    inspector.locator(":focus"),
    "Shift+Tab from the first control must wrap to the last control inside the inspector",
  ).toHaveCount(1);
  await expect(closeButton).not.toBeFocused();
  await closeButton.focus();
  await page.keyboard.press("Escape");
  await expect(inspector).toBeHidden();
  await expect(evidenceTrigger).toBeFocused();
  await expect(page.locator(".product-main")).not.toHaveAttribute("inert", "");

  const workspaceTrigger = page.getByRole("button", { name: "Open workspaces" });
  await workspaceTrigger.click();
  const workspaceDrawer = page.getByRole("dialog", { name: "Workspaces" });
  await expect(workspaceDrawer).toHaveAttribute("role", "dialog");
  await expect(workspaceDrawer).toHaveAttribute("aria-modal", "true");
  await expect(workspaceDrawer.getByRole("button", { name: "Close workspaces" })).toBeFocused();
  await expect(
    workspaceDrawer.getByRole("searchbox", { name: "Search workspaces and sessions" }),
  ).toBeVisible();
  await expect(
    workspaceDrawer.getByText("Search workspaces and sessions", { exact: true }),
  ).toHaveCount(0);
  const lastDrawerAction = workspaceDrawer.getByRole("button", { name: "Product settings" });
  await lastDrawerAction.focus();
  await page.keyboard.press("Tab");
  await expect(workspaceDrawer.getByRole("button", { name: "Add workspace" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(workspaceDrawer).toBeHidden();
  await expect(workspaceTrigger).toBeFocused();

  await page.keyboard.press("/");
  await expect(page.getByRole("textbox", { name: "Message" })).toBeFocused();
  const motion = await page.locator(".product-root").evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      animationDuration: style.animationDuration,
      transitionDuration: style.transitionDuration,
    };
  });
  expect(Number.parseFloat(motion.animationDuration)).toBeLessThanOrEqual(0.001);
  expect(Number.parseFloat(motion.transitionDuration)).toBeLessThanOrEqual(0.001);

  await page.screenshot({
    path: testInfo.outputPath("mobile-chat-light.png"),
    fullPage: true,
  });

  await page.setViewportSize({ width: 320, height: 800 });
  await expectNoHorizontalOverflow(page);
  const narrowComposerBox = await page
    .getByRole("textbox", { name: "Message" })
    .boundingBox();
  expect(narrowComposerBox?.width).toBeGreaterThan(200);
  await page.screenshot({
    path: testInfo.outputPath("mobile-chat-320.png"),
    fullPage: true,
  });
});

test("mobile migration summary stays outside the composer", async ({
  page,
}, testInfo) => {
  await installMockProductApi(page);
  await seedLegacyCatalog(page);

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Imported mobile session" })).toBeVisible();
  const summary = page.getByText("Browser data imported (2 records).");
  await expect(summary).toBeVisible();
  await expectNoHorizontalOverflow(page);

  const summaryBox = await summary.locator("xpath=ancestor::div[1]").boundingBox();
  const composerBox = await page.locator(".chat-composer").boundingBox();
  expect(summaryBox).not.toBeNull();
  expect(composerBox).not.toBeNull();
  expect(rectanglesOverlap(summaryBox!, composerBox!)).toBe(false);

  await page.screenshot({
    path: testInfo.outputPath("mobile-migration-summary.png"),
    fullPage: true,
  });
});

test("workspace dialog traps focus, closes with Escape, and restores its trigger", async ({
  page,
}, testInfo) => {
  await installMockProductApi(page);
  await page.goto("/");

  await page.getByRole("button", { name: "Open workspaces" }).click();
  const workspaceDrawer = page.getByRole("dialog", { name: "Workspaces" });
  const trigger = page.getByRole("button", { name: "Add workspace" });
  await trigger.click();
  const dialog = page.getByRole("dialog", { name: "Open workspace" });
  await expect(dialog).toHaveAttribute("aria-modal", "true");
  const pathInput = dialog.getByLabel("Absolute path");
  await expect(pathInput).toBeFocused();

  await page.keyboard.press("Shift+Tab");
  await expect(dialog.getByRole("button", { name: "Open", exact: true })).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(pathInput).toBeFocused();
  const focusStyle = await pathInput.evaluate((element) => {
    const style = getComputedStyle(element);
    return { color: style.outlineColor, width: style.outlineWidth };
  });
  expect(focusStyle.width).toBe("2px");

  await dialog.getByRole("button", { name: "Open", exact: true }).click();
  await expect(dialog.getByRole("alert")).toHaveText("Enter an absolute path.");
  await expect(pathInput).toHaveAttribute("aria-invalid", "true");
  await page.screenshot({
    path: testInfo.outputPath("mobile-workspace-dialog.png"),
    fullPage: true,
  });

  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(trigger).toBeFocused();
  await workspaceDrawer.getByRole("button", { name: "Close workspaces" }).click();
});

test("server-confirmed dark theme and deep Settings tab survive reload", async ({
  page,
}, testInfo) => {
  const api = await installMockProductApi(page);
  await page.goto("/settings/general");
  await page.getByRole("button", { name: "Dark", exact: true }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await expect
    .poll(() => browserValue(page, SERVER_CONFIRMED_THEME_CACHE_KEY))
    .toBe("dark");

  await page.goto("/settings/about");
  const activeTab = page.getByRole("button", { name: "About / Runtime" });
  await expect(activeTab).toHaveAttribute("aria-current", "page");
  const tabVisible = await activeTab.evaluate((element) => {
    const tab = element.getBoundingClientRect();
    const nav = element.parentElement!.getBoundingClientRect();
    return tab.left >= nav.left && tab.right <= nav.right;
  });
  expect(tabVisible).toBe(true);
  await expect(page.getByRole("heading", { name: "Resume health" })).toBeVisible();
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  expect(api.migrationRequestBodies).toHaveLength(0);

  await page.screenshot({
    path: testInfo.outputPath("mobile-settings-dark.png"),
    fullPage: true,
  });

  await page.route(/\/api\/product(?:\/.*)?(?:\?.*)?$/, (route) =>
    route.abort("failed"),
  );
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
});

test("strict Mermaid rendering preserves visible SVG text labels", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const workspace = createMockWorkspace();
  const session = createMockSession();
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: {
      [session.id]: completedTranscript(
        workspace,
        session,
        "Show the execution path",
        "```mermaid\nflowchart LR\n  A[Plan] --> B[Execute]\n```",
      ),
    },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  const diagram = page.getByRole("figure", { name: "Mermaid diagram" });
  await expect(diagram).toBeVisible();
  await expect(diagram.locator("foreignObject")).toHaveCount(0);
  const labels = diagram.locator("svg text");
  const planLabel = labels.filter({ hasText: "Plan" });
  const executeLabel = labels.filter({ hasText: "Execute" });
  await expect(planLabel).toHaveCount(1);
  await expect(planLabel).toBeVisible();
  await expect(executeLabel).toHaveCount(1);
  await expect(executeLabel).toBeVisible();

  await page.screenshot({
    path: testInfo.outputPath("desktop-mermaid-labels.png"),
    fullPage: true,
  });
});

async function seedLegacyCatalog(page: Page) {
  const values: Record<string, string> = {
    [M1_BROWSER_STORAGE_KEYS.workspaces]: JSON.stringify([
      {
        id: "mobile-workspace",
        rootPath: "D:/tmp/rove-mobile-migration",
        kind: "folder",
        displayName: "Imported mobile workspace",
        pinned: false,
        lastOpenedAt: "2026-07-27T00:00:00.000Z",
      },
    ]),
    [M1_BROWSER_STORAGE_KEYS.sessions]: JSON.stringify([
      {
        id: "mobile-session",
        workspaceId: "mobile-workspace",
        title: "Imported mobile session",
        createdAt: "2026-07-27T00:00:00.000Z",
        updatedAt: "2026-07-27T00:00:00.000Z",
        status: "idle",
        hasDurableTurn: false,
      },
    ]),
    [M1_BROWSER_STORAGE_KEYS.active]: JSON.stringify({
      workspaceId: "mobile-workspace",
      sessionId: "mobile-session",
    }),
  };
  await page.addInitScript((entries) => {
    for (const [key, value] of Object.entries(entries)) {
      window.localStorage.setItem(key, value);
    }
  }, values);
}

async function expectNoHorizontalOverflow(page: Page) {
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          document.documentElement.scrollWidth <=
          document.documentElement.clientWidth,
      ),
    )
    .toBe(true);
}

function rectanglesOverlap(
  first: { x: number; y: number; width: number; height: number },
  second: { x: number; y: number; width: number; height: number },
): boolean {
  return !(
    first.x + first.width <= second.x ||
    second.x + second.width <= first.x ||
    first.y + first.height <= second.y ||
    second.y + second.height <= first.y
  );
}

async function browserValue(page: Page, key: string): Promise<string | null> {
  return page.evaluate((storageKey) => window.localStorage.getItem(storageKey), key);
}
