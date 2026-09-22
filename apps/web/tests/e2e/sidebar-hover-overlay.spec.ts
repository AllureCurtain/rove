import { expect, test, type Locator, type Page } from "@playwright/test";

import {
  completedTranscript,
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
} from "./product-api-mock";

/**
 * Narrow-screen rail peek, ported from open-vetta's hover-summoned overlay
 * (`openOverlay` / `scheduleOverlayClose`).
 *
 * The peek is deliberately *not* the modal drawer: touching the left edge shows
 * the rail without making it a dialog and without moving focus, and it hides
 * again after a short grace period. The header button stays the deliberate,
 * keyboard-reachable way in.
 */
const NARROW = { width: 900, height: 900 };
const CLOSE_GRACE_MS = 120;

async function openSession(page: Page, viewport = NARROW) {
  await page.setViewportSize(viewport);
  const workspace = createMockWorkspace();
  const session = createMockSession();
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: {
      [session.id]: completedTranscript(workspace, session, "Peek question", "Peek answer"),
    },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(page.getByText("Peek answer", { exact: true })).toBeVisible();
  return { workspace, session };
}

function drawer(page: Page): Locator {
  return page.locator("aside.product-sidebar");
}

test("the left edge summons the rail without taking focus", async ({ page }) => {
  await openSession(page);
  const rail = drawer(page);
  const zone = page.locator(".sidebar-hover-zone");

  await expect(rail).toHaveAttribute("data-open", "false");
  await expect(zone).toBeVisible();
  const zoneBox = await zone.boundingBox();
  expect(zoneBox?.width, "the summon strip hugs the left edge").toBeLessThanOrEqual(16);

  // Focus stays wherever it was: a pointer gesture must not move it.
  await page.getByRole("textbox", { name: /输入消息/ }).focus();
  await zone.hover();
  await expect(rail).toHaveAttribute("data-open", "true");
  await expect(rail).toBeVisible();
  expect(
    await page.evaluate(() => document.activeElement?.tagName ?? ""),
    "the peek must not steal focus",
  ).not.toBe("BUTTON");

  // A peek is not a dialog: no modal semantics, so the page behind stays usable.
  await expect(rail).not.toHaveAttribute("role", "dialog");
  await expect(rail).not.toHaveAttribute("aria-modal", "true");
  await expect(rail).toHaveAttribute("data-peek", "true");

  // Crossing out of the rail closes it.
  await page.mouse.move(700, 500);
  await expect(rail).toHaveAttribute("data-open", "false");

  // Leaving and coming back inside the grace period keeps it open instead of
  // flickering: this is the reference's scheduleOverlayClose/openOverlay pair.
  await zone.hover();
  await expect(rail).toHaveAttribute("data-open", "true");
  await page.mouse.move(700, 500);
  await page.mouse.move(60, 500);
  await page.waitForTimeout(CLOSE_GRACE_MS + 140);
  await expect(
    rail,
    "the pointer returned inside the grace period, so the rail stays open",
  ).toHaveAttribute("data-open", "true");
  await page.mouse.move(700, 500);
  await expect(rail).toHaveAttribute("data-open", "false");
});

test("a peek closes on selection without moving focus; the button still moves it", async ({
  page,
}) => {
  await openSession(page);
  const rail = drawer(page);
  const heading = page.getByRole("heading", { level: 1 });

  // Pointer path: select a session from a peek.
  await page.locator(".sidebar-hover-zone").hover();
  await expect(rail).toHaveAttribute("data-peek", "true");
  await rail.locator(".session-item").first().click();
  await expect(rail).toHaveAttribute("data-open", "false");
  await expect(
    heading,
    "a pointer-driven peek must not pull focus into the conversation heading",
  ).not.toBeFocused();

  // Deliberate path: the header button opens the modal drawer, which does.
  await page.getByRole("button", { name: "展开工作区" }).click();
  await expect(rail).toHaveAttribute("role", "dialog");
  await expect(rail).not.toHaveAttribute("data-peek", "true");
  await rail.locator(".session-item").first().click();
  await expect(rail).toHaveAttribute("data-open", "false");
  await expect(heading).toBeFocused();
});

test("the summon strip does not exist on a wide screen", async ({ page }) => {
  await openSession(page, { width: 1440, height: 900 });
  await expect(page.locator(".sidebar-hover-zone")).toHaveCount(0);
});
