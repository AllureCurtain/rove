import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

import {
  createMockSession,
  createMockWorkspace,
  installMockProductApi,
  longCompletedTranscript,
} from "./product-api-mock";

/**
 * WCAG checks for the product shell, which had none.
 *
 * The shell is the surface a person actually operates: a rail, a transcript, a
 * composer, an inspector and the Settings pages. Its accessibility claims were
 * asserted in prose and in a handful of hand-written role/name expectations
 * (`shell.spec.ts`, `polish.spec.ts`), which can only confirm the attributes
 * somebody already thought of. `@axe-core/playwright` runs the maintained WCAG
 * rule engine — colour contrast, names, roles, landmarks, heading order, ARIA
 * validity, target size — against a rendered page, which is the part a
 * hand-written assertion cannot cover.
 *
 * The rule set is stated rather than implied: WCAG 2.0 A/AA plus 2.1 A/AA, the
 * level this product commits to. Each tone is checked in every appearance the
 * product ships (`rove.ui-skin` warm/graphite × light/dark), because a token is
 * only verified in the theme it was measured in.
 *
 * One violation is accepted, below, with its reason and the alternatives that
 * were rejected. It is keyed by rule id and target and only accepted on the
 * surfaces that render the tab strip: the same rule firing anywhere else, or on
 * a surface without the strip, still fails, and if the strip stops producing it
 * the case that renders the strip fails too, so the exception cannot outlive
 * its cause.
 */

const VIEWPORT = { width: 1280, height: 720 };

/** The levels this product commits to, as axe tag names. */
const WCAG_TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"];

/** Where the shell keeps the chosen skin; the theme is `html[data-theme]`. */
const UI_SKIN_STORAGE_KEY = "rove.ui-skin";

/**
 * The work panel is closed by default (design §3.3), so the checks that cover
 * its tab strip seed the persisted open flag — the same state a reload
 * restores after a user opens the panel.
 */
const WORK_PANEL_OPEN_STORAGE_KEY = "rove.ui-work-panel-open";

/** Persist the work panel as open before the app boots. */
async function seedPanelOpen(page: Page) {
  await page.addInitScript(
    (key) => window.localStorage.setItem(key, "1"),
    WORK_PANEL_OPEN_STORAGE_KEY,
  );
}

/**
 * The work-panel tab strip pairs each tab with its own close button inside the
 * tab list. ARIA's `tablist` allows owned `tab` children only, so axe reports
 * the close button as an unallowed child.
 *
 * Both structures that satisfy the rule were rejected: moving the close button
 * out of the tab list takes it away from the tab it closes and out of the strip
 * a keyboard user is traversing, and rendering it as a non-focusable span
 * removes a named control from the accessibility tree (`layout.spec.ts` asserts
 * it is reachable by name). Keeping the control the design intends costs one
 * documented deviation; the alternative costs a real affordance.
 */
const ACCEPTED_TAB_STRIP_CLOSE_BUTTON = {
  id: "aria-required-children",
  target: ".inspector-tabs",
  reason:
    "the tab list owns each tab's close button, which ARIA does not allow as a child of `tablist`",
} as const;

/**
 * Run axe over the current page and fail with the rule ids, the impact and the
 * offending markup, which is what makes a failure actionable without a
 * screenshot. For a contrast failure the check's own data (foreground,
 * background, ratio, required ratio) is included, because "which colour" is the
 * whole question.
 *
 * `options.tabStrip` states whether this surface renders the work-panel tab
 * strip: it is the only place where the accepted violation above may appear,
 * and where it must appear.
 */
async function expectNoViolations(
  page: Page,
  surface: string,
  options: { tabStrip?: boolean } = {},
) {
  const results = await new AxeBuilder({ page }).withTags(WCAG_TAGS).analyze();
  const undocumented: unknown[] = [];
  let accepted = 0;
  for (const violation of results.violations) {
    for (const node of violation.nodes) {
      const target = node.target.join(" ");
      const entry = {
        id: violation.id,
        impact: violation.impact,
        help: violation.help,
        target,
        detail: node.any[0]?.data ?? undefined,
      };
      const isAccepted =
        options.tabStrip === true &&
        violation.id === ACCEPTED_TAB_STRIP_CLOSE_BUTTON.id &&
        target.includes(ACCEPTED_TAB_STRIP_CLOSE_BUTTON.target);
      if (isAccepted) {
        accepted += 1;
        continue;
      }
      undocumented.push(entry);
    }
  }
  expect(
    undocumented,
    `${surface}: WCAG violations\n${JSON.stringify(undocumented, null, 2)}`,
  ).toEqual([]);
  // Self-cleaning: the exception exists only while the strip produces it.
  expect(
    accepted > 0,
    `${surface}: the documented tab-strip exception (${ACCEPTED_TAB_STRIP_CLOSE_BUTTON.id}) no longer applies; remove it`,
  ).toBe(options.tabStrip === true);
}

const COMPOSER = /输入消息/u;

/** The appearances the product ships, so a token is verified where it is used. */
const APPEARANCES = [
  { label: "warm skin, light theme", skin: "warm", theme: "light" },
  { label: "warm skin, dark theme", skin: "warm", theme: "dark" },
  { label: "graphite skin, light theme", skin: "graphite", theme: "light" },
  { label: "graphite skin, dark theme", skin: "graphite", theme: "dark" },
] as const;

type Appearance = (typeof APPEARANCES)[number];

/**
 * Sessions whose status badges paint the warning, danger and attention tones.
 * These three tones are small text on their own tint, which is exactly the pair
 * that failed contrast before `styles/v3/tokens.css` made the tints opaque.
 */
function statusSessions(workspaceId: string) {
  return [
    { ...createMockSession("session-running", workspaceId, "Running session"), status: "running" as const },
    { ...createMockSession("session-error", workspaceId, "Failed session"), status: "error" as const },
    {
      ...createMockSession("session-attention", workspaceId, "Waiting session"),
      status: "needs_attention" as const,
    },
  ];
}

const WORKSPACE = createMockWorkspace();
const SESSION = createMockSession("session-1", WORKSPACE.id, "Durable session");

/** The shell with a restored 30-run transcript: rail, chat, transcript, inspector. */
async function openRestoredSession(page: Page, sessions = [SESSION]) {
  await seedPanelOpen(page);
  await installMockProductApi(page, {
    workspaces: [WORKSPACE],
    sessions,
    transcripts: { [SESSION.id]: longCompletedTranscript(WORKSPACE, SESSION, 3) },
    activeWorkspaceId: WORKSPACE.id,
    activeSessionId: SESSION.id,
  });
  await page.goto(`/w/${WORKSPACE.id}/s/${SESSION.id}`);
  await expect(page.getByText("Restored answer 3", { exact: true })).toBeVisible();
}

/** Put the shell into one shipped appearance before asserting on it. */
async function applyAppearance(page: Page, appearance: Appearance) {
  // The shipped default is graphite, so a warm check has to write its own key
  // too; the skin under test must never depend on whichever default ships.
  await page.addInitScript(
    ([key, skin]) => window.localStorage.setItem(key, skin),
    [UI_SKIN_STORAGE_KEY, appearance.skin] as const,
  );
  await openRestoredSession(page, [SESSION, ...statusSessions(WORKSPACE.id)]);
  await expect(page.locator(".product-app-frame")).toHaveAttribute(
    "data-skin",
    appearance.skin,
  );
  if (appearance.theme === "dark") {
    await page.getByRole("button", { name: "切换到深色主题" }).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  }
}

test("the empty shell has no WCAG violations", async ({ page }) => {
  await page.setViewportSize(VIEWPORT);
  await installMockProductApi(page);
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "打开工作区以开始" })).toBeVisible();
  await expectNoViolations(page, "empty shell");
});

for (const appearance of APPEARANCES) {
  test(`a restored session in the ${appearance.label} has no WCAG violations`, async ({
    page,
  }) => {
    await page.setViewportSize(VIEWPORT);
    await applyAppearance(page, appearance);
    await expectNoViolations(page, `restored session (${appearance.label})`, {
      tabStrip: true,
    });

    // The inspector is a second region in the same view; its collapsed state is
    // the other arrangement a reader navigates. Checked once, in the appearance
    // the product defaults to. Scoped to the inspector, because the panel's own
    // close button carries the same label.
    if (appearance.theme === "light" && appearance.skin === "warm") {
      await page
        .getByRole("complementary", { name: "详情" })
        .getByLabel("收起详情面板")
        .click();
      // Collapsing the inspector removes the tab strip, so the documented
      // exception must not appear here: that is the other direction of the
      // check, and it is why this case declares no tab strip.
      await expectNoViolations(page, "session with the inspector collapsed");
    }
  });
}

test("the composer with a draft and its notices has no WCAG violations", async ({ page }) => {
  await page.setViewportSize(VIEWPORT);
  await openRestoredSession(page);
  const composer = page.getByRole("textbox", { name: COMPOSER });
  await composer.fill("a draft that is not sent");
  // The keyboard-shortcut hint and the character counter only exist once a draft
  // does, so they are checked in the state that renders them.
  await expectNoViolations(page, "composer with a draft", { tabStrip: true });
});

test("the Settings sections have no WCAG violations", async ({ page }) => {
  await page.setViewportSize(VIEWPORT);
  await installMockProductApi(page);
  await page.goto("/settings/general");
  await expect(page.getByRole("heading", { name: /通用|常规/u })).toBeVisible();
  await expectNoViolations(page, "settings general");

  await page.goto("/settings/providers");
  await expect(page.getByRole("heading", { name: "模型服务" })).toBeVisible();
  await expectNoViolations(page, "settings providers");
});

test("a live approval and its acknowledgement have no WCAG violations", async ({ page }) => {
  await page.setViewportSize(VIEWPORT);
  await seedPanelOpen(page);
  await installMockProductApi(page, { mode: "approval" });
  await page.goto("/");
  await page.getByLabel("绝对路径").fill("D:/tmp/rove-a11y");
  await page.getByRole("button", { name: "打开工作区", exact: true }).click();
  const composer = page.getByRole("textbox", { name: COMPOSER });
  await expect(composer).toBeVisible();
  await composer.fill("Write a note");
  await composer.press("Control+Enter");
  await expect(page.getByLabel("审批")).toBeVisible();
  await expectNoViolations(page, "session with a pending approval", { tabStrip: true });
});
