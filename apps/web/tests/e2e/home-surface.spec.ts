import { expect, test } from "@playwright/test";

import {
  installMockProductApi,
  type MockSession,
  type MockWorkspace,
} from "./product-api-mock";

/**
 * §9.1: the middle column without a session is the home surface — hero, the
 * home variant of the same composer, an inline workspace switcher, recents,
 * and the demoted manual path entry. §9.2: with no satisfiable provider the
 * composer stays editable but the send is blocked with a direct action.
 */

const ALPHA: MockWorkspace = {
  id: "ws-alpha",
  canonical_root: "D:/code/alpha",
  kind: "repo",
  display_name: "alpha",
  pinned: false,
  last_opened_at: "2026-10-01T08:00:00Z",
  created_at: "2026-09-30T08:00:00Z",
  updated_at: "2026-10-01T08:00:00Z",
};

const BETA: MockWorkspace = {
  id: "ws-beta",
  canonical_root: "D:/code/beta",
  kind: "folder",
  display_name: "beta",
  pinned: false,
  last_opened_at: "2026-10-02T08:00:00Z",
  created_at: "2026-09-30T09:00:00Z",
  updated_at: "2026-10-02T08:00:00Z",
};

const RECENT: MockSession = {
  id: "session-recent",
  workspace_id: "ws-alpha",
  title: "Reconcile the ledger",
  status: "idle",
  created_at: "2026-10-03T08:00:00Z",
  updated_at: "2026-10-04T09:30:00Z",
  last_outcome: "success",
  last_outcome_at: "2026-10-04T09:30:00Z",
};

test("home shows the hero, the home composer, and a demoted manual entry", async ({
  page,
}) => {
  await installMockProductApi(page);
  await page.goto("/");

  await expect(
    page.getByRole("heading", { name: "让 rove 做点什么？" }),
  ).toBeVisible();
  await expect(
    page.getByRole("textbox", { name: "描述交给 rove 的任务…" }),
  ).toBeVisible();
  // The absolute-path form is present but collapsed behind the disclosure.
  const manual = page.locator("details.home-surface__manual");
  await expect(manual).toBeAttached();
  await expect(manual).not.toHaveAttribute("open", "");
  await expect(manual.getByLabel("绝对路径")).toBeHidden();
});

test("the hero switcher and the sidebar share one open-workspace dialog", async ({
  page,
}) => {
  await installMockProductApi(page, { workspaces: [ALPHA] });
  await page.goto("/");

  // The dotted-underline project name opens the picker menu (§9.1).
  await page.locator(".home-surface__switcher-button").click();
  const menu = page.getByRole("menu", { name: "工作区选择" });
  await expect(menu.getByRole("menuitemradio", { name: "alpha" })).toBeVisible();
  await menu.getByRole("menuitem", { name: "打开文件夹或仓库…" }).click();
  const dialog = page.getByRole("dialog", { name: "打开工作区" });
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);

  // The sidebar `+` mounts the same dialog — one selection flow (§9.1).
  await page.getByRole("button", { name: "添加工作区" }).first().click();
  await expect(page.getByRole("dialog", { name: "打开工作区" })).toBeVisible();
});

test("a home send creates the session and delivers the first message", async ({
  page,
}) => {
  const api = await installMockProductApi(page, {
    workspaces: [ALPHA],
    activeWorkspaceId: ALPHA.id,
  });
  await page.goto("/");

  const composer = page.getByRole("textbox", {
    name: "描述交给 rove 的任务…",
  });
  await composer.fill("Summarize the runtime state");
  await composer.press("Enter");

  // The send is armed before navigation and fires once the new session is
  // focused — one gesture, no lost first message.
  await expect(page).toHaveURL(/\/w\/ws-alpha\/s\/session-\d+$/u);
  const conversation = page.getByTestId("conversation-log");
  await expect(
    conversation.getByText("Summarize the runtime state"),
  ).toBeVisible();
  await expect.poll(() => api.jobs.length).toBe(1);
  expect(api.jobs[0]?.product_session_id).toBe(api.sessions[0]?.id);
});

test("recents keep the past one click away", async ({ page }) => {
  await installMockProductApi(page, {
    workspaces: [ALPHA, BETA],
    sessions: [RECENT],
  });
  // A workspace route without a session of its own lands on the home surface;
  // the recents list still reaches across to alpha's session.
  await page.goto("/w/ws-beta");

  const recent = page
    .locator(".home-surface__recent")
    .filter({ hasText: "Reconcile the ledger" });
  await expect(recent).toContainText("alpha");
  await recent.click();

  await expect(page).toHaveURL(/\/w\/ws-alpha\/s\/session-recent$/u);
});

test("the switcher menu switches the hero's workspace", async ({ page }) => {
  await installMockProductApi(page, {
    workspaces: [ALPHA, BETA],
    activeWorkspaceId: ALPHA.id,
  });
  await page.goto("/");

  await page.locator(".home-surface__switcher-button").click();
  await page
    .getByRole("menu", { name: "工作区选择" })
    .getByRole("menuitemradio", { name: "beta" })
    .click();

  await expect(page).toHaveURL(/\/w\/ws-beta$/u);
  await expect(
    page.locator(".home-surface__switcher-button"),
  ).toContainText("beta");
});

test("manual path entry still opens a workspace behind the disclosure", async ({
  page,
}) => {
  const api = await installMockProductApi(page);
  await page.goto("/");

  await page.getByText("手动输入路径").click();
  await page.getByLabel("绝对路径").fill("D:/tmp/rove-home-manual");
  await page.getByLabel("类型").selectOption("repo");
  await page
    .locator(".home-surface__manual")
    .getByRole("button", { name: "打开工作区" })
    .click();

  await expect(page).toHaveURL(/\/w\/workspace-1\/s\/session-1$/u);
  expect(api.workspaces[0]).toMatchObject({
    canonical_root: "D:/tmp/rove-home-manual",
    kind: "repo",
  });
});

test("a missing provider profile blocks the home send with a direct action", async ({
  page,
}) => {
  const api = await installMockProductApi(page, {
    workspaces: [ALPHA],
    activeWorkspaceId: ALPHA.id,
    providerSelection: {
      profile_id: "profile-gone",
      model: "fake",
      approval: "ask",
      max_steps: 8,
    },
  });
  await page.goto("/");

  const composer = page.getByRole("textbox", {
    name: "描述交给 rove 的任务…",
  });
  const send = page.getByRole("button", { name: "发送", exact: true });
  await expect(composer).toBeEnabled();
  await expect(send).toBeDisabled();
  const hint = page.locator(".home-surface__send-hint");
  await expect(hint).toContainText("发送前请先配置模型服务。");

  // §9.2: an actual submit still answers — the hint escalates and carries the
  // direct action, and no session or job is created.
  await composer.fill("Ship it anyway");
  await composer.press("Enter");
  await expect(hint).toHaveAttribute("role", "alert");
  await hint.getByRole("button", { name: "模型服务设置" }).click();
  await expect(page).toHaveURL(/settings\/providers/u);
  expect(api.sessions).toHaveLength(0);
  expect(api.jobs).toHaveLength(0);
});
