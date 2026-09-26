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
    main.x,
    "the collapsed rail must not hold a column",
  ).toBeLessThanOrEqual(1);
  // The rail itself still has to leave the row: since F9 the track snaps in one
  // frame and the rail slides out on its own transform, so its box is only gone
  // once that slide has run.
  await expect
    .poll(async () => {
      const settledRail = await box(page, ".product-sidebar");
      return settledRail.x + settledRail.width;
    })
    .toBeLessThanOrEqual(1);
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
 * F9 (design §10): collapsing the rail must not animate the grid track.
 *
 * Animating `grid-template-columns` re-lays-out the conversation column on every
 * frame — measured at 1280x720 with a 30-run transcript, one collapse cost 15
 * layout passes and 22.5ms of layout plus 34.2ms of style recalculation, against
 * 2 passes / 1.9ms / 5.7ms once the track settles in one frame. The rail keeps
 * its own 240ms transform slide instead, which is why it also needs a stable
 * width: as a stretched grid item in a 0 track it would collapse in the same
 * frame and have nothing left to slide.
 */
test("the rail's collapse snaps the track and slides the rail itself", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 800 });
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

  const motion = await page.evaluate(() => {
    const body = document.querySelector(".product-body");
    const rail = document.querySelector(".product-sidebar");
    return {
      bodyTransition: body ? getComputedStyle(body).transitionProperty : "",
      railTransition: rail ? getComputedStyle(rail).transitionProperty : "",
      railWidth: rail ? getComputedStyle(rail).width : "",
      railPosition: rail ? getComputedStyle(rail).position : "",
    };
  });
  expect(
    motion.bodyTransition,
    "the grid track must settle in one frame",
  ).not.toContain("grid-template-columns");
  expect(
    motion.railTransition,
    "the rail keeps its own transform slide",
  ).toContain("transform");
  expect(motion.railWidth).toBe("240px");
  expect(motion.railPosition).toBe("relative");

  const body = page.locator(".product-body");
  await page.getByRole("button", { name: "收起工作区列表" }).click();
  await expect(body).toHaveAttribute("data-nav-collapsed", "true");

  // The conversation owns the row in the same frame the rail starts sliding, and
  // the rail keeps a full-width box painted over it while it leaves.
  const main = await box(page, ".product-main");
  expect(main.x, "conversation starts at the viewport edge").toBeCloseTo(0, 0);
  const collapsedRail = await page.evaluate(() => {
    const rail = document.querySelector(".product-sidebar");
    return {
      width: rail ? getComputedStyle(rail).width : "",
      zIndex: rail ? getComputedStyle(rail).zIndex : "",
      ariaHidden: rail?.getAttribute("aria-hidden"),
      inert: rail?.hasAttribute("inert") ?? false,
    };
  });
  expect(collapsedRail.width).toBe("240px");
  expect(collapsedRail.zIndex).toBe("2");
  // The off-screen rail leaves the accessibility tree and the tab order.
  expect(collapsedRail.ariaHidden).toBe("true");
  expect(collapsedRail.inert).toBe(true);

  // Reopening restores the rail's column, again without animating the track:
  // the conversation starts at the rail's edge immediately, and the rail itself
  // slides back in from the left.
  await page.getByRole("button", { name: "展开工作区列表" }).click();
  await expect(body).toHaveAttribute("data-nav-collapsed", "false");
  const reopenedRail = await box(page, ".product-sidebar");
  const reopenedMain = await box(page, ".product-main");
  expect(reopenedRail.width).toBeCloseTo(240, 0);
  expect(reopenedMain.x).toBeCloseTo(reopenedRail.width, 0);
  await expect
    .poll(async () => (await box(page, ".product-sidebar")).x)
    .toBeCloseTo(0, 0);
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

  // Delete closes the focused tab and focuses the neighbour. Focus is asserted
  // first, because tab focus moves in a frame after the arrow key.
  await page.keyboard.press("ArrowRight");
  await expect(filesTab).toBeFocused();
  await expect(filesTab).toHaveAttribute("aria-selected", "true");
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
  // An empty strip must not leave invalid ARIA behind: `tablist` requires owned
  // tabs and a `tabpanel` needs the tab that labels it.
  await expect(page.getByRole("tablist")).toHaveCount(0);
  await expect(page.getByRole("tabpanel")).toHaveCount(0);
  await openTabButton.click();
  await expect(launcherTab).toBeVisible();
});

/**
 * Design §3.3: the conversation has one reading measure. The transcript column
 * and the composer must share both edges, the measure must stay capped, and at
 * desktop sizes the conversation column must keep its 680px comfort width.
 */
const READING_COLUMN_VIEWPORTS = [
  { width: 1440, height: 900, desktop: true },
  { width: 1280, height: 800, desktop: true },
  { width: 375, height: 812, desktop: false },
];

for (const viewport of READING_COLUMN_VIEWPORTS) {
  test(`conversation keeps one reading measure at ${viewport.width}x${viewport.height}`, async ({
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
          "Reading measure question",
          "Reading measure answer",
        ),
      },
      activeWorkspaceId: workspace.id,
      activeSessionId: session.id,
    });
    await page.goto(`/w/${workspace.id}/s/${session.id}`);
    await expect(page.getByText("Reading measure answer", { exact: true })).toBeVisible();

    const main = await box(page, ".product-main");
    const content = await box(page, ".chat-transcript__content");
    const composer = await box(page, ".chat-composer");

    if (viewport.desktop) {
      expect(main.width, "conversation keeps its 680px comfort width").toBeGreaterThanOrEqual(
        680,
      );
    }
    expect(content.width, "reading measure is capped at 840px").toBeLessThanOrEqual(840);
    expect(
      Math.abs(content.width - composer.width),
      "composer shares the transcript measure",
    ).toBeLessThanOrEqual(1);
    expect(
      Math.abs(content.x - composer.x),
      "composer shares the transcript left edge",
    ).toBeLessThanOrEqual(1);
    // Centered inside the conversation column.
    const leftGap = content.x - main.x;
    const rightGap = main.x + main.width - (content.x + content.width);
    expect(Math.abs(leftGap - rightGap), "reading measure is centered").toBeLessThanOrEqual(2);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
  });
}

/**
 * The rail handle must resize the rail it sits on.
 *
 * `settleWidth` used to measure from the handle's own rect, but the handle is
 * pinned to the rail's trailing edge and slides with the rail: grabbing a 240px
 * rail computed `240 - 236 = 4px`, which snapped the rail to its 200px minimum
 * on the first pointer move and then kept re-measuring from the moved handle.
 * Measured on the pre-fix build at 1440x900: a +40px drag left the rail at 200.
 */
test("dragging the rail handle resizes the rail and keeps the columns", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const workspace = createMockWorkspace();
  const session = createMockSession();
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: {
      [session.id]: completedTranscript(workspace, session, "Rail question", "Rail answer"),
    },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(page.getByText("Rail answer", { exact: true })).toBeVisible();

  const before = await box(page, ".product-sidebar");
  expect(before.width, "the rail starts at its 240px default").toBe(240);

  const handle = await box(page, ".sidebar-resize-handle");
  const startX = handle.x + handle.width / 2;
  const y = handle.y + 60;
  await page.mouse.move(startX, y);
  await page.mouse.down();
  for (let step = 1; step <= 4; step += 1) {
    await page.mouse.move(startX + step * 10, y);
  }
  await page.mouse.up();

  const grown = await box(page, ".product-sidebar");
  expect(
    grown.width,
    "the rail must follow the pointer instead of snapping to its minimum",
  ).toBeGreaterThanOrEqual(276);
  expect(grown.width, "the drag must not overshoot").toBeLessThanOrEqual(284);

  // The columns must survive the resize: conversation still on the floor, panel
  // still on the first row and still flush with the viewport edge.
  const main = await box(page, ".product-main");
  const panel = await box(page, "aside.product-inspector");
  expect(main.x, "the conversation starts where the rail ends").toBeCloseTo(
    grown.width,
    0,
  );
  expect(main.width).toBeGreaterThanOrEqual(MAIN_PANE_MIN_WIDTH);
  expect(panel.y, "the work panel stays on the first row").toBeCloseTo(main.y, 0);
  expect(panel.x + panel.width, "the panel stays flush with the viewport").toBeCloseTo(
    1440,
    0,
  );
  expect(
    await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),
  ).toBe(true);
});

/**
 * Keyboard resizing must keep working next to the pointer path: the handle is a
 * `role="separator"` with a bounded value, not a decorative grip.
 */
test("the rail handle resizes from the keyboard within its bounds", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const workspace = createMockWorkspace();
  const session = createMockSession();
  await installMockProductApi(page, {
    workspaces: [workspace],
    sessions: [session],
    transcripts: {
      [session.id]: completedTranscript(workspace, session, "Rail question", "Rail answer"),
    },
    activeWorkspaceId: workspace.id,
    activeSessionId: session.id,
  });
  await page.goto(`/w/${workspace.id}/s/${session.id}`);
  await expect(page.getByText("Rail answer", { exact: true })).toBeVisible();

  const handle = page.getByRole("separator", { name: /导航宽度|sidebar width/ });
  await handle.focus();
  await handle.press("ArrowRight");
  await expect(handle).toHaveAttribute("aria-valuenow", "256");
  await handle.press("Shift+ArrowRight");
  await expect(handle).toHaveAttribute("aria-valuenow", "288");
  // The rail width is animated, so the rendered box converges rather than
  // snapping: poll it instead of reading a frame of the easing tail.
  await expect
    .poll(async () => (await box(page, ".product-sidebar")).width, {
      message: "the rail converges on the width the handle announced",
    })
    .toBeCloseTo(288, 0);
  await handle.press("Home");
  await expect(handle).toHaveAttribute("aria-valuenow", "200");
  await handle.press("ArrowLeft");
  await expect(handle).toHaveAttribute("aria-valuenow", "200");
  await handle.press("End");
  await expect(handle).toHaveAttribute("aria-valuenow", "360");
  await expect
    .poll(async () => (await box(page, ".product-sidebar")).width, {
      message: "End converges on the maximum rail width",
    })
    .toBeCloseTo(360, 0);
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
