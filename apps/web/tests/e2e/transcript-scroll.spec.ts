import { expect, test, type Page } from "@playwright/test";

import {
  completedTranscript,
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
} from "./product-api-mock";

/**
 * Transcript follow behaviour (PI-Desktop `useFollowScroll`).
 *
 * The interesting cases are the ones a threshold-only implementation gets wrong:
 * a gesture releases follow, a layout correction does not, and the jump control
 * brings the reader back to the newest turn.
 */

const LONG_ANSWER = Array.from(
  { length: 160 },
  (_value, index) => `restored answer line ${index + 1}`,
).join("\n\n");

async function openLongTranscript(page: Page) {
  await page.setViewportSize({ width: 1280, height: 800 });
  const workspace = createMockWorkspace();
  const session = createMockSession();
  const transcript = completedTranscript(
    workspace,
    session,
    "Restored question",
    LONG_ANSWER,
  );
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: { [session.id]: transcript },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  const scroller = page.getByLabel("Conversation");
  await expect(scroller.getByText("restored answer line 160")).toBeVisible();
  return scroller;
}

async function distanceFromBottom(scroller: ReturnType<Page["getByLabel"]>) {
  return scroller.evaluate((node) => {
    const element = node as HTMLElement;
    return Math.max(
      0,
      element.scrollHeight - element.scrollTop - element.clientHeight,
    );
  });
}

test("a restored transcript opens pinned to its newest turn", async ({ page }) => {
  const scroller = await openLongTranscript(page);

  await expect.poll(() => distanceFromBottom(scroller)).toBeLessThanOrEqual(2);
  await expect(page.locator("button.return-to-latest")).toHaveCount(0);
});

test("a wheel gesture up releases follow and reveals the scroller while it moves", async ({
  page,
}) => {
  const scroller = await openLongTranscript(page);
  await scroller.hover();

  await page.mouse.wheel(0, -700);

  const jump = page.getByRole("button", { name: /回到最新|Return to latest/ });
  await expect(jump).toBeVisible();
  // Reveal-while-scrolling marks whatever scrolled, and clears once it is quiet.
  await expect(scroller).toHaveAttribute("data-scrolling", "");
  await expect(scroller).not.toHaveAttribute("data-scrolling", "", {
    timeout: 5_000,
  });
});

test("the jump control returns to the newest turn and then removes itself", async ({
  page,
}) => {
  const scroller = await openLongTranscript(page);
  await scroller.hover();
  await page.mouse.wheel(0, -700);

  const jump = page.getByRole("button", { name: /回到最新|Return to latest/ });
  await expect(jump).toBeVisible();
  await jump.click();

  await expect.poll(() => distanceFromBottom(scroller)).toBeLessThanOrEqual(2);
  await expect(page.locator("button.return-to-latest")).toHaveCount(0);
});

test("a programmatic scroll correction does not release follow", async ({ page }) => {
  const scroller = await openLongTranscript(page);

  // A follow `scrollTo` or a resize clamp moves the scroller without any input
  // gesture behind it. Reading that as the user scrolling up would drop follow
  // mode for the rest of the turn.
  await scroller.evaluate((node) => {
    const element = node as HTMLElement;
    element.scrollTop = Math.max(0, element.scrollTop - 400);
  });

  await expect.poll(() => distanceFromBottom(scroller)).toBeLessThanOrEqual(2);
  await expect(page.locator("button.return-to-latest")).toHaveCount(0);
});
