import { expect, test, type Locator, type Page } from "@playwright/test";

import {
  completedTranscript,
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
} from "./product-api-mock";

/**
 * Conversation reading band handles, ported from PI-Desktop D439.
 *
 * At 1440x900 the conversation column is 840px and the transcript pads 16px per
 * side, so the band can use 808px: `min(available, preferred)` with the 840px
 * design measure as the preference. Both handles move one centered band, so a
 * 1px pointer move is a 2px width change.
 */
const COLUMN_WIDTH = 840;
const BAND_AVAILABLE = COLUMN_WIDTH - 32;

async function openSession(page: Page, width = 1440, height = 900) {
  await page.setViewportSize({ width, height });
  const workspace = createMockWorkspace();
  const session = createMockSession();
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: {
      [session.id]: completedTranscript(workspace, session, "Band question", "Band answer"),
    },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(page.getByText("Band answer", { exact: true })).toBeVisible();
}

async function box(page: Page, selector: string) {
  const locator: Locator = page.locator(selector);
  const rect = await locator.boundingBox();
  expect(rect, `${selector} must be laid out`).not.toBeNull();
  return rect!;
}

/** Drags a handle horizontally; the band width changes by twice the distance. */
async function dragHandle(page: Page, handle: Locator, deltaX: number) {
  const rect = await handle.boundingBox();
  expect(rect, "handle must be laid out").not.toBeNull();
  const startX = rect!.x + rect!.width / 2;
  const y = rect!.y + 80;
  await page.mouse.move(startX, y);
  await page.mouse.down();
  for (let step = 1; step <= 4; step += 1) {
    await page.mouse.move(startX + (deltaX / 4) * step, y);
  }
  await page.mouse.up();
}

test("the reading band resizes from either edge and keeps one measure", async ({
  page,
}) => {
  await openSession(page);
  const handles = page.getByRole("separator", { name: /阅读宽度/ });
  const left = handles.first();

  // The band is `min(available, preferred)`, so the 840px measure renders at the
  // 808px the padded column can actually use.
  await expect(handles).toHaveCount(2);
  await expect(left).toHaveAttribute("aria-valuemin", "560");
  await expect(left).toHaveAttribute("aria-valuemax", String(BAND_AVAILABLE));
  await expect(left).toHaveAttribute("aria-valuenow", String(BAND_AVAILABLE));
  await expect(left).toHaveAttribute("aria-valuetext", /808/);

  const content = await box(page, ".chat-transcript__content");
  const composer = await box(page, ".chat-composer");
  expect(content.width).toBeCloseTo(BAND_AVAILABLE, 0);
  expect(content.width).toBeCloseTo(composer.width, 0);

  // The handles straddle the band edges: this is what the CSS min()/max()
  // arithmetic has to produce, so a dropped declaration cannot pass silently.
  const bandLeft = content.x;
  const bandRight = content.x + content.width;
  const leftBox = await box(page, '.chat-reading-handle[data-side="left"]');
  const rightBox = await box(page, '.chat-reading-handle[data-side="right"]');
  expect(leftBox.x + leftBox.width / 2).toBeCloseTo(bandLeft, 0);
  expect(rightBox.x + rightBox.width / 2).toBeCloseTo(bandRight, 0);
  expect(leftBox.width).toBe(12);

  // Dragging inward narrows the band by twice the pointer distance.
  await dragHandle(page, left, 20);
  await expect(left).toHaveAttribute("aria-valuenow", String(BAND_AVAILABLE - 40));
  const narrowed = await box(page, ".chat-transcript__content");
  const narrowedComposer = await box(page, ".chat-composer");
  expect(narrowed.width).toBeCloseTo(BAND_AVAILABLE - 40, 0);
  expect(narrowedComposer.width).toBeCloseTo(narrowed.width, 0);
  expect(
    Math.abs(narrowed.x - bandLeft) -
      Math.abs(bandRight - (narrowed.x + narrowed.width)),
    "the band stays centered",
  ).toBeLessThanOrEqual(2);

  // The preference survives a reload; the band comes back at the same width.
  await page.reload({ waitUntil: "load" });
  await expect(page.getByText("Band answer", { exact: true })).toBeVisible();
  const reloaded = page.getByRole("separator", { name: /阅读宽度/ }).first();
  await expect(reloaded).toHaveAttribute("aria-valuenow", String(BAND_AVAILABLE - 40));
  expect((await box(page, ".chat-transcript__content")).width).toBeCloseTo(
    BAND_AVAILABLE - 40,
    0,
  );

  // Double-click returns to the design measure.
  await reloaded.dblclick();
  await expect(reloaded).toHaveAttribute("aria-valuenow", String(BAND_AVAILABLE));

  // Keyboard: arrows step toward or away from the centre per edge, Home resets,
  // End spends the whole column.
  await reloaded.focus();
  await page.keyboard.press("ArrowRight");
  await expect(reloaded).toHaveAttribute("aria-valuenow", String(BAND_AVAILABLE - 16));
  await page.keyboard.press("Shift+ArrowRight");
  await expect(reloaded).toHaveAttribute("aria-valuenow", String(BAND_AVAILABLE - 48));
  await page.keyboard.press("Home");
  await expect(reloaded).toHaveAttribute("aria-valuenow", String(BAND_AVAILABLE));
  await page.keyboard.press("ArrowLeft");
  await expect(
    reloaded,
    "the left handle widens toward the column edge, capped by the design measure",
  ).toHaveAttribute("aria-valuenow", String(BAND_AVAILABLE));
  await page.keyboard.press("End");
  await expect(reloaded).toHaveAttribute("aria-valuenow", String(BAND_AVAILABLE));

  // The band must not have dragged the rest of the shell with it.
  await expect(page.getByRole("separator", { name: /面板宽度/ })).toHaveCount(1);
  const panel = await box(page, "aside.product-inspector");
  expect(
    panel.x + panel.width,
    "the work panel stays flush with the viewport",
  ).toBeCloseTo(1440, 0);
  const main = await box(page, ".product-main");
  expect(main.width).toBeGreaterThanOrEqual(450);
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
    1440,
  );
});

test("the band handles disappear when the column has no room to give", async ({
  page,
}) => {
  // 960 is the drawer boundary and 375 is a phone: the band is the whole column
  // there, so there is nothing to move.
  for (const width of [960, 375]) {
    await openSession(page, width, width < 500 ? 812 : 900);
    await expect(
      page.getByRole("separator", { name: /阅读宽度/ }),
      `no band handles at ${width}`,
    ).toHaveCount(0);
    const content = await box(page, ".chat-transcript__content");
    const composer = await box(page, ".chat-composer");
    expect(content.width).toBeCloseTo(composer.width, 0);
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth),
    ).toBeLessThanOrEqual(width);
  }
});
