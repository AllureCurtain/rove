import { expect, test, type Page } from "@playwright/test";

import {
  completedTranscript,
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
} from "./product-api-mock";

/**
 * Width sweep for the shell's geometric invariants, including the lane the
 * conversation minimap needs.
 *
 * The one-off probe that found the original grid defects printed numbers by hand;
 * this is the permanent version, so a future change to the panel/rail budget or to
 * the reading column cannot quietly break the relationships the shell is designed
 * around. Relationships, not pixel snapshots: the point is the arithmetic, and the
 * two minimap cases are the boundaries that actually decide whether the rail has
 * somewhere to live.
 */

const READING_CAP_PX = 840;
/** Rail offset (26px) + rail width (20px), rounded up to the next clean number. */
const MINIMAP_LANE_PX = 50;

/**
 * This file measures settled geometry five times per case, and the first request
 * against a cold `next dev` server also pays the route compile. The 30s suite default
 * is sized for interaction cases, so a loaded machine could run out of budget while
 * every measurement was still correct; give the measurement sweep its own bounded
 * budget instead of failing on timing.
 */
test.describe.configure({ timeout: 90_000 });

/**
 * The rail yields to the conversation floor with a transition, so a measurement
 * taken while the shell is still settling reads an intermediate frame width. Wait
 * for two consecutive identical samples of the columns before trusting anything.
 */
async function waitForSettledColumns(page: Page) {
  await expect
    .poll(
      async () =>
        page.evaluate(() => {
          const body = document.querySelector<HTMLElement>(".product-body");
          return body ? getComputedStyle(body).gridTemplateColumns : "";
        }),
      { timeout: 15_000, intervals: [100, 150, 200, 250] },
    )
    .not.toBe("");
  let previous = "";
  for (let attempt = 0; attempt < 20; attempt += 1) {
    const current = await page.evaluate(() => {
      const body = document.querySelector<HTMLElement>(".product-body");
      const main = document.querySelector<HTMLElement>(".product-main");
      return `${body ? getComputedStyle(body).gridTemplateColumns : ""}|${main?.clientWidth ?? 0}`;
    });
    if (current === previous) {
      return;
    }
    previous = current;
    await page.waitForTimeout(120);
  }
}

const OVERFLOWING_ANSWER = Array.from(
  { length: 60 },
  (_value, index) => `geometry answer line ${index + 1}`,
).join("\n\n");

async function openShell(page: Page, width: number) {
  const workspace = createMockWorkspace();
  const session = createMockSession();
  // A long answer on purpose: the rail is only *eligible* once the transcript
  // overflows, so a short transcript would make the lane assertions vacuous.
  const transcript = completedTranscript(
    workspace,
    session,
    "Geometry question",
    OVERFLOWING_ANSWER,
  );
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: { [session.id]: transcript },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  await page.setViewportSize({ width, height: 900 });
  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(page.getByRole("button", { name: /发送|Send/ })).toBeVisible();
  return { workspace, session };
}

async function shellGeometry(page: Page) {
  return page.evaluate(() => {
    const box = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) {
        return null;
      }
      const rect = element.getBoundingClientRect();
      return {
        left: Math.round(rect.left),
        right: Math.round(rect.right),
        width: Math.round(rect.width),
      };
    };
    const frame = document.querySelector<HTMLElement>(".chat-transcript-frame");
    const column = document.querySelector<HTMLElement>(".chat-transcript__content");
    const rail = document.querySelector<HTMLElement>(".conversation-minimap");
    const columnBox = box(".chat-transcript__content");
    return {
      overflowX:
        document.documentElement.scrollWidth - document.documentElement.clientWidth,
      frameWidth: frame?.clientWidth ?? null,
      columnWidth: column ? Math.round(column.getBoundingClientRect().width) : null,
      columnBox,
      railBox: box(".conversation-minimap"),
      railVisible: rail ? getComputedStyle(rail).display !== "none" : false,
      railMarkers: document.querySelectorAll(".conversation-minimap__marker").length,
    };
  });
}

test("the reading column and the minimap lane keep their arithmetic across widths", async ({
  page,
}) => {
  const measured: Array<Record<string, unknown>> = [];

  for (const width of [1024, 1280, 1440, 1600, 1920]) {
    await openShell(page, width);
    await waitForSettledColumns(page);
    const geometry = await shellGeometry(page);
    const frameWidth = geometry.frameWidth;
    const columnWidth = geometry.columnWidth;
    expect(frameWidth).not.toBeNull();
    expect(columnWidth).not.toBeNull();

    // No width may overflow the shell horizontally.
    expect(geometry.overflowX, `overflow at ${width}`).toBe(0);

    // One reading measure: the column is the cap, or the frame minus its 16px
    // padding on both sides — never wider than the cap.
    const expectedColumn = Math.min(READING_CAP_PX, (frameWidth ?? 0) - 32);
    expect(columnWidth, `column at ${width}`).toBeGreaterThanOrEqual(expectedColumn - 1);
    expect(columnWidth, `column at ${width}`).toBeLessThanOrEqual(expectedColumn + 1);
    expect(columnWidth, `column at ${width}`).toBeLessThanOrEqual(READING_CAP_PX);

    const gutter = Math.round(((frameWidth ?? 0) - (columnWidth ?? 0)) / 2);
    // Two independent gates, asserted separately: the rail is *rendered* because
    // the transcript overflows, and it is *shown* only when a lane exists.
    const railRendered = geometry.railMarkers > 0;
    const railShown = geometry.railVisible;
    if (gutter >= MINIMAP_LANE_PX) {
      // The pane is wider than the cap, so the rail has its own lane and must use
      // it: outside the reading column, so it can never sit on prose.
      expect(railRendered, `markers at ${width}`).toBe(true);
      expect(railShown, `rail at ${width}`).toBe(true);
      const railBox = geometry.railBox;
      const columnBox = geometry.columnBox;
      expect(railBox).not.toBeNull();
      expect(columnBox).not.toBeNull();
      expect(
        (railBox?.left ?? 0) >= (columnBox?.right ?? 0) ||
          (railBox?.right ?? 0) <= (columnBox?.left ?? 0),
        `rail overlaps the reading column at ${width}`,
      ).toBe(true);
    } else {
      // No lane: the rail must stand down rather than overlap prose.
      expect(railShown, `rail at ${width}`).toBe(false);
    }

    measured.push({ width, ...geometry, gutter, expectedColumn });
  }

  // The sweep is only meaningful if it actually crossed the boundary both ways.
  const gutters = measured.map((row) => row.gutter as number);
  expect(gutters.some((gutter) => gutter < MINIMAP_LANE_PX)).toBe(true);
  expect(gutters.some((gutter) => gutter >= MINIMAP_LANE_PX)).toBe(true);
});

test("the rail hides on a narrow pane instead of covering the conversation", async ({ page }) => {
  // 1024x768 is the narrowest desktop band the layout suite pins: the pane is far
  // below the reading cap, so there is no lane and no rail.
  await openShell(page, 1024);
  await waitForSettledColumns(page);
  const geometry = await shellGeometry(page);
  expect(geometry.railMarkers).toBeGreaterThan(0);
  expect(geometry.railVisible).toBe(false);
  expect(geometry.overflowX).toBe(0);
  // The conversation itself is still usable: the transcript scrolls.
  const scrollable = await page
    .getByLabel("Conversation")
    .evaluate((node) => (node as HTMLElement).scrollHeight > 0);
  expect(scrollable).toBe(true);
});
