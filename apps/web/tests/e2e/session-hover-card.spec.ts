import { expect, test } from "@playwright/test";

import {
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
} from "./product-api-mock";

/**
 * Session hover card (design F4).
 *
 * The card is informational and pointer-only: it appears after the row has been
 * hovered for the delay, disappears when the pointer leaves, and never opens
 * from keyboard focus. Its one lazy read is the session model config, which must
 * happen once per session.
 *
 * Each case hovers a session that is *not* the active one: the shell reads the
 * active session's model config on load, and the card's own read is what these
 * assertions are about.
 */

const CARD = ".session-hover-card";

function readsFor(api: { sessionModelConfigReads: string[] }, sessionId: string): number {
  return api.sessionModelConfigReads.filter((id) => id === sessionId).length;
}

test("hovering a session row reveals its details card", async ({ page }) => {
  const workspace = createMockWorkspace();
  const active = createMockSession("session-active", workspace.id, "Active session");
  const hovered = createMockSession("session-hovered", workspace.id, "Hovered session");
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [active, hovered],
    activeWorkspaceId: workspace.id,
    activeSessionId: active.id,
  });

  await page.goto(`/w/${workspace.id}/s/${active.id}`);
  const row = page.locator(".session-item").filter({ hasText: "Hovered session" });
  await expect(row).toBeVisible();

  // Nothing appears until the row has been hovered for the delay.
  await expect(page.locator(CARD)).toHaveCount(0);
  await row.hover();
  await expect(page.locator(CARD)).toBeVisible();

  const card = page.locator(CARD);
  await expect(card).toHaveAttribute("role", "tooltip");
  await expect(card.locator(".session-hover-card__title")).toHaveText(hovered.title);
  await expect(card).toContainText("排队中");
  await expect(card).toContainText("2026-07-26 00:00:00 UTC");
  await expect(card).toContainText(workspace.canonical_root);

  // The model row arrived from the lazy read: model, reasoning, step limit.
  await expect(card).toContainText("fake");
  await expect(card).toContainText("服务默认");
  await expect(card).toContainText("8");
  expect(readsFor(api, hovered.id)).toBe(1);

  // The card lives inside the shell (tokens and scoped styles), not in <body>.
  await expect(page.locator(`.product-app-frame ${CARD}`)).toBeVisible();
  // ...and it never intercepts the row it describes.
  await expect(card).toHaveCSS("pointer-events", "none");

  // The row points at the card while it is shown.
  await expect(row).toHaveAttribute("aria-describedby", `session-hover-card-${hovered.id}`);

  // Leaving the row closes it.
  await page.mouse.move(900, 500);
  await expect(page.locator(CARD)).toHaveCount(0);
});

test("keyboard focus never opens the card", async ({ page }) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await page.locator(".session-item").first().focus();
  // Longer than the hover delay: focus is not a hover.
  await page.waitForTimeout(600);
  await expect(page.locator(CARD)).toHaveCount(0);
});

test("Escape closes the card and it stays closed while the pointer rests there", async ({
  page,
}) => {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });

  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  const row = page.locator(".session-item").first();
  await row.hover();
  await expect(page.locator(CARD)).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(page.locator(CARD)).toHaveCount(0);

  // Still hovering the same row: it must not come straight back.
  await page.waitForTimeout(600);
  await expect(page.locator(CARD)).toHaveCount(0);

  // Leaving and hovering again works.
  await page.mouse.move(900, 500);
  await row.hover();
  await expect(page.locator(CARD)).toBeVisible();
});

test("a failed model read shows a dash and is not retried on the next hover", async ({ page }) => {
  const workspace = createMockWorkspace();
  const active = createMockSession("session-active", workspace.id, "Active session");
  const hovered = createMockSession("session-hovered", workspace.id, "Hovered session");
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [active, hovered],
    activeWorkspaceId: workspace.id,
    activeSessionId: active.id,
    sessionModelConfigReadFailures: { [hovered.id]: 1 },
  });

  await page.goto(`/w/${workspace.id}/s/${active.id}`);
  const row = page.locator(".session-item").filter({ hasText: "Hovered session" });
  await row.hover();

  const card = page.locator(CARD);
  await expect(card).toBeVisible();
  // The read failed, so the model row states nothing rather than a guess.
  await expect(
    card.locator(".session-hover-card__fact").filter({ hasText: "模型" }),
  ).toContainText("—");
  expect(readsFor(api, hovered.id)).toBe(1);

  // A failed read is remembered: re-hovering does not retry it.
  await page.mouse.move(900, 500);
  await expect(page.locator(CARD)).toHaveCount(0);
  await row.hover();
  await expect(page.locator(CARD)).toBeVisible();
  await page.waitForTimeout(400);
  expect(readsFor(api, hovered.id)).toBe(1);
});
