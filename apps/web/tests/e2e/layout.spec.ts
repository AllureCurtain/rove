import { expect, test, type Locator, type Page } from "@playwright/test";

import { WORK_PANEL_DEFAULT_WIDTH } from "../../inspector/work-panel-layout";
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
  { width: 1440, height: 900, railExpanded: true },
  { width: 1280, height: 800, railExpanded: true },
  // 1024 − 240 (rail) − 360 (panel) = 424 < 450, so the rail yields first
  // instead of the conversation being squeezed (design §5.0 priority order).
  { width: 1024, height: 768, railExpanded: false },
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
    const expectedHeight = viewport.height - TOPBAR_HEIGHT;

    // One row: every visible column starts under the top bar and fills the
    // viewport height.
    for (const [name, element] of [
      ["conversation", main],
      ["work panel", panel],
      ...(viewport.railExpanded ? ([["rail", rail]] as const) : []),
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
    if (viewport.railExpanded) {
      expect(main.x, "conversation must start where the rail ends").toBeCloseTo(
        rail.x + rail.width,
        0,
      );
    } else {
      // The rail yields first rather than squeezing the conversation.
      await expect(page.locator(".product-body")).toHaveAttribute(
        "data-nav-collapsed",
        "true",
      );
    }
    expect(
      main.x + main.width,
      "conversation must end where the work panel starts",
    ).toBeCloseTo(panel.x, 0);

    // The work panel is the right column, not a second-row box on the left.
    expect(panel.x, "work panel must sit right of the rail").toBeGreaterThan(
      0,
    );
    expect(
      panel.x + panel.width,
      "work panel must end at the viewport edge",
    ).toBeCloseTo(viewport.width, 0);

    // The rail handle overlays the rail's trailing edge instead of taking a track.
    if (viewport.railExpanded) {
      const handle = await box(page, ".sidebar-resize-handle");
      expect(
        handle.x,
        "handle must overlay the rail's trailing edge",
      ).toBeGreaterThanOrEqual(rail.x + rail.width - 8);
      expect(
        handle.x,
        "handle must not consume the conversation column",
      ).toBeLessThan(main.x + 1);
    }

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
 * Design §5.0 priority order — conversation floor > panel request > rail — so at
 * 1024x768 the rail yields instead of the conversation dropping to 424px, and
 * reopening the rail spends the panel's column rather than the chat's.
 */
test("the rail yields before the conversation floor at 1024x768", async ({
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

  const body = page.locator(".product-body");
  await expect(body).toHaveAttribute("data-nav-collapsed", "true");
  const collapsedRail = await box(page, ".product-sidebar");
  const main = await box(page, ".product-main");
  const panel = await box(page, "aside.product-inspector");

  expect(
    collapsedRail.x + collapsedRail.width,
    "the collapsed rail must not hold a column",
  ).toBeLessThanOrEqual(1);
  expect(
    main.width,
    "conversation keeps its 450px floor",
  ).toBeGreaterThanOrEqual(MAIN_PANE_MIN_WIDTH);
  expect(panel.width, "panel keeps its stored width").toBeCloseTo(
    WORK_PANEL_DEFAULT_WIDTH,
    0,
  );
  expect(main.x + main.width + panel.width).toBeCloseTo(1024, 0);

  // Reopening the rail spends the panel's column, not the conversation's.
  await page.getByRole("button", { name: "展开工作区列表" }).click();
  await expect(body).toHaveAttribute("data-nav-collapsed", "false");
  const reopenedMain = await box(page, ".product-main");
  const reopenedPanel = await box(page, "aside.product-inspector");
  expect(
    reopenedMain.width,
    "conversation keeps its floor after reopening the rail",
  ).toBeGreaterThanOrEqual(MAIN_PANE_MIN_WIDTH);
  expect(
    reopenedPanel.width,
    "the panel gives up the space the rail took",
  ).toBeLessThan(WORK_PANEL_DEFAULT_WIDTH);
});

/**
 * P2: the work panel is a dynamic, closable tab strip (ported from PI-Desktop
 * `WorkPanel`), not a fixed set of tabs. The strip must open a launcher page,
 * replace it with the chosen tab, close tabs by button/middle-click/Delete,
 * keep focus and selection coherent, and offer a way back when empty.
 */
test("work panel tab strip opens, closes and reopens tabs", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const workspace = createMockWorkspace();
  const session = createMockSession();
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  await page.goto(`/w/${workspace.id}/s/${session.id}`);

  const strip = page.getByRole("tablist", { name: "详情页签" });
  const statusTab = strip.getByRole("tab", { name: "运行", exact: true });
  const filesTab = strip.getByRole("tab", { name: "文件", exact: true });
  const launcherTab = strip.getByRole("tab", { name: "打开页签", exact: true });
  const openTabButton = page.getByRole("button", { name: "打开一个页签", exact: true });

  // A fresh session starts with exactly the run status tab.
  await expect(statusTab).toHaveAttribute("aria-selected", "true");
  await expect(strip.getByRole("tab")).toHaveCount(1);
  await expect(page.getByRole("tabpanel")).toHaveAttribute(
    "aria-labelledby",
    "inspector-tab-status",
  );

  // `+` opens a launcher page; choosing a kind replaces that page.
  const panel = page.getByRole("tabpanel");
  await openTabButton.click();
  await expect(launcherTab).toBeVisible();
  await expect(strip.getByRole("tab")).toHaveCount(2);
  await panel.getByRole("button", { name: "文件", exact: true }).click();
  await expect(filesTab).toHaveAttribute("aria-selected", "true");
  await expect(launcherTab).toHaveCount(0);
  await expect(strip.getByRole("tab")).toHaveCount(2);

  // Roving tabindex: only the selected tab is tabbable, arrows move selection.
  await expect(filesTab).toHaveAttribute("tabindex", "0");
  await expect(statusTab).toHaveAttribute("tabindex", "-1");
  await statusTab.focus();
  await page.keyboard.press("ArrowRight");
  await expect(filesTab).toBeFocused();
  await expect(filesTab).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("ArrowLeft");
  await expect(statusTab).toBeFocused();
  await expect(statusTab).toHaveAttribute("aria-selected", "true");

  // Delete closes the focused tab and focuses the neighbour.
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("Delete");
  await expect(filesTab).toHaveCount(0);
  await expect(statusTab).toHaveAttribute("aria-selected", "true");
  await expect(statusTab).toBeFocused();

  // Middle-click closes a tab too.
  await openTabButton.click();
  await panel.getByRole("button", { name: "变更", exact: true }).click();
  const changesTab = strip.getByRole("tab", { name: "变更", exact: true });
  await expect(changesTab).toHaveAttribute("aria-selected", "true");
  await changesTab.click({ button: "middle" });
  await expect(changesTab).toHaveCount(0);

  // The explicit close button works, and an empty strip keeps a way back.
  await openTabButton.click();
  await panel.getByRole("button", { name: "文件", exact: true }).click();
  await strip.getByRole("button", { name: "关闭 文件", exact: true }).click();
  await expect(filesTab).toHaveCount(0);
  await strip.getByRole("button", { name: "关闭 运行", exact: true }).click();
  await expect(strip.getByRole("tab")).toHaveCount(0);
  await expect(page.getByText("没有打开的页签，用 + 打开一个。", { exact: true })).toBeVisible();
  await openTabButton.click();
  await expect(launcherTab).toBeVisible();
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
