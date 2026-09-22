import { expect, test, type Locator, type Page } from "@playwright/test";

import {
  completedTranscript,
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
} from "./product-api-mock";

/**
 * Top bar height used by the shell (`--topbar-height` in product-v2.css).
 */
const TOPBAR_HEIGHT = 52;
/**
 * Design §5.0: the conversation column is the hard floor of the shared width
 * budget. Reaching it must shrink the panel, never the conversation.
 */
const MAIN_PANE_MIN_WIDTH = 450;

/**
 * The three-column shell contract: rail | conversation | work panel share one
 * row, and each column fills the viewport below the top bar.
 *
 * Regression for the `sidebar-resize-handle` grid-track bug: as a grid item in
 * column 2 it consumed the middle track, which pushed `<main>` into column 3
 * (auto-sized) and moved the inspector onto a second row, bottom-left. At
 * 1440x900 the rail, main and inspector were each only 424px tall, main started
 * at x=538 and the inspector at y=476 — a visibly broken page rather than a
 * styling preference.
 */
const DESKTOP_VIEWPORTS = [
  { width: 1440, height: 900 },
  { width: 1280, height: 800 },
  { width: 1024, height: 768 },
];

for (const viewport of DESKTOP_VIEWPORTS) {
  test(`three-column shell stays on one row at ${viewport.width}x${viewport.height}`, async ({
    page,
  }) => {
    await page.setViewportSize(viewport);
    const workspace = createMockWorkspace();
    const session = createMockSession();
    await installMockProductApi(page, {
      workspaces: [workspace],
      sessions: [session],
      transcripts: {
        [session.id]: completedTranscript(
          workspace,
          session,
          "Layout question",
          "Layout answer",
        ),
      },
      activeWorkspaceId: workspace.id,
      activeSessionId: session.id,
    });

    await page.goto(`/w/${workspace.id}/s/${session.id}`);
    await expect(page.getByText("Layout answer", { exact: true })).toBeVisible();

    const inspector = page.locator("aside.product-inspector");
    // Above 960px the panel is a column, not the modal drawer.
    await expect(inspector).not.toHaveAttribute("role", "dialog");

    const rail = await box(page, ".product-sidebar");
    const main = await box(page, ".product-main");
    const panel = await box(page, "aside.product-inspector");
    const handle = await box(page, ".sidebar-resize-handle");
    const expectedHeight = viewport.height - TOPBAR_HEIGHT;

    // One row: all three columns start under the top bar and fill the viewport.
    for (const [name, element] of [
      ["rail", rail],
      ["conversation", main],
      ["work panel", panel],
    ] as const) {
      expect(element.y, `${name} must start below the top bar`).toBeCloseTo(
        TOPBAR_HEIGHT,
        0,
      );
      expect(
        element.height,
        `${name} must fill the viewport height`,
      ).toBeCloseTo(expectedHeight, 0);
    }

    // No empty grid track between the rail and the conversation.
    expect(main.x, "conversation must start where the rail ends").toBeCloseTo(
      rail.x + rail.width,
      0,
    );
    expect(
      main.x + main.width,
      "conversation must end where the work panel starts",
    ).toBeCloseTo(panel.x, 0);

    // The work panel is the right column, not a second-row box on the left.
    expect(panel.x, "work panel must sit right of the rail").toBeGreaterThan(
      rail.x + rail.width,
    );
    expect(
      panel.x + panel.width,
      "work panel must end at the viewport edge",
    ).toBeCloseTo(viewport.width, 0);

    // The resize handle overlays the rail's trailing edge instead of taking a track.
    expect(
      handle.x,
      "handle must overlay the rail's trailing edge",
    ).toBeGreaterThanOrEqual(rail.x + rail.width - 8);
    expect(handle.x, "handle must not consume the conversation column").toBeLessThan(
      main.x + 1,
    );

    expect(
      main.width,
      "conversation must fill the space between the rail and the panel",
    ).toBeCloseTo(viewport.width - rail.width - panel.width, 0);
    if (viewport.width - rail.width - panel.width >= MAIN_PANE_MIN_WIDTH) {
      expect(
        main.width,
        "conversation must keep the design's 450px floor when there is room",
      ).toBeGreaterThanOrEqual(MAIN_PANE_MIN_WIDTH);
    }

    const scrollWidth = await page.evaluate(
      () => document.documentElement.scrollWidth,
    );
    expect(scrollWidth, "the shell must not overflow horizontally").toBeLessThanOrEqual(
      viewport.width,
    );
  });
}

/**
 * Design §5.0 shared width budget: the conversation column is the first claim on
 * the width, so at 1024x768 the work panel shrinks to `1024 − rail − 450`
 * instead of squeezing the conversation down to 424px.
 */
test("the work panel yields to the 450px conversation floor at 1024x768", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1024, height: 768 });
  const workspace = createMockWorkspace();
  const session = createMockSession();
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: {
      [session.id]: completedTranscript(workspace, session, "Layout question", "Layout answer"),
    },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(page.getByText("Layout answer", { exact: true })).toBeVisible();

  const rail = await box(page, ".product-sidebar");
  const main = await box(page, ".product-main");
  const panel = await box(page, "aside.product-inspector");

  expect(rail.width + main.width + panel.width).toBeCloseTo(1024, 0);
  expect(
    main.width,
    "conversation keeps its 450px floor",
  ).toBeGreaterThanOrEqual(MAIN_PANE_MIN_WIDTH);
  expect(
    panel.width,
    "panel shrinks to fit the budget",
  ).toBeLessThanOrEqual(1024 - rail.width - MAIN_PANE_MIN_WIDTH);
  expect(panel.width, "panel does not collapse to nothing").toBeGreaterThan(200);
});

async function box(
  page: Page,
  selector: string,
): Promise<{ x: number; y: number; width: number; height: number }> {
  const locator: Locator = page.locator(selector);
  const box = await locator.boundingBox();
  expect(box, `${selector} must be laid out`).not.toBeNull();
  return box!;
}
