import { expect, test } from "@playwright/test";

const route = "/dev/product-ui-v2";

test("Product UI V2 switches among independent mock sessions", async ({ page }) => {
  const apiRequests: string[] = [];
  page.on("request", (request) => {
    if (new URL(request.url()).pathname.startsWith("/api/")) {
      apiRequests.push(request.url());
    }
  });

  await page.goto(route);
  await expect(page.locator('meta[name="robots"]')).toHaveAttribute(
    "content",
    /noindex.*nofollow/,
  );
  await expect(page.getByRole("note")).toHaveText(
    "Inert design mock. No API, persistence, or real approvals.",
  );

  const cases = [
    {
      id: "session-c4-web-control",
      title: "C4 web control surface",
      transcript: "The first implementation slice should repair workspace-scoped Memory",
      inspector: "Current execution",
      dateTime: "2026-07-27T09:43:00+08:00",
    },
    {
      id: "session-memory-scope-audit",
      title: "Memory scope audit",
      transcript: "The workspace, user, and runtime layers resolve independently.",
      inspector: "Audit result",
      dateTime: "2026-07-27T09:25:00+08:00",
    },
    {
      id: "session-provider-retry",
      title: "Provider retry behavior",
      transcript: "The retry remains fail closed.",
      inspector: "Retry boundary",
      dateTime: "2026-07-27T09:01:00+08:00",
    },
  ] as const;

  for (const [index, item] of cases.entries()) {
    const sessionButton = page.locator(`[data-session-id="${item.id}"]`).first();
    if (index === 1) {
      await sessionButton.click();
    } else if (index === 2) {
      await sessionButton.focus();
      await page.keyboard.press("Enter");
    }

    await expect(sessionButton).toHaveAttribute("aria-current", "page");
    await expect(page.locator('[aria-current="page"]')).toHaveCount(1);
    await expect(page.getByRole("heading", { level: 1, name: item.title })).toBeVisible();
    await expect(page.getByText(item.transcript, { exact: false })).toBeVisible();
    await expect(page.getByLabel("Run evidence").getByText(item.inspector, { exact: true })).toBeVisible();
    await expect(sessionButton.locator("time")).toHaveAttribute("datetime", item.dateTime);
  }

  await expect(page.getByText("Inert UI mock", { exact: true })).toBeVisible();
  expect(apiRequests).toEqual([]);
});

test("desktop workspace rail releases Tab into the main surface", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(route);

  const rail = page.locator('aside[aria-label="Workspaces and sessions"]');
  await expect(rail).not.toHaveAttribute("role", "dialog");
  await expect(rail).not.toHaveAttribute("aria-modal", "true");

  await rail.getByRole("button", { name: "Product settings" }).focus();
  await page.keyboard.press("Tab");

  await expect(page.getByRole("button", { name: "Session actions" })).toBeFocused();
});

test("mobile workspace drawer traps focus and restores its trigger", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(route);

  const trigger = page.getByRole("button", { name: "Open workspaces" });
  await trigger.click();

  const drawer = page.locator('aside[aria-label="Workspaces and sessions"]');
  const closeButton = drawer.getByRole("button", { name: "Close workspaces" });
  const firstButton = drawer.getByRole("button", { name: "Open workspace" });
  const lastButton = drawer.getByRole("button", { name: "Product settings" });
  await expect(drawer).toHaveAttribute("role", "dialog");
  await expect(drawer).toHaveAttribute("aria-modal", "true");
  await expect(closeButton).toBeFocused();

  await lastButton.focus();
  await page.keyboard.press("Tab");
  await expect(firstButton).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(lastButton).toBeFocused();

  await page.keyboard.press("Escape");
  await expect(drawer).toBeHidden();
  await expect(trigger).toBeFocused();
});

test("mobile evidence sheet traps focus and restores its trigger", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(route);

  const trigger = page.getByRole("button", { name: "Open run evidence" });
  await trigger.click();

  const inspector = page.locator('aside[aria-label="Run evidence"]');
  const closeButton = inspector.getByRole("button", { name: "Close run evidence" });
  await expect(inspector).toHaveAttribute("role", "dialog");
  await expect(inspector).toHaveAttribute("aria-modal", "true");
  await expect(closeButton).toBeFocused();

  await page.keyboard.press("Tab");
  await expect(closeButton).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(closeButton).toBeFocused();

  await page.keyboard.press("Escape");
  await expect(inspector).toBeHidden();
  await expect(trigger).toBeFocused();
});

test("mobile session selection closes the drawer and moves focus to the new heading", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(route);

  await page.getByRole("button", { name: "Open workspaces" }).click();
  const drawer = page.locator('aside[aria-label="Workspaces and sessions"]');
  await expect(drawer).toHaveAttribute("role", "dialog");
  const sessionButton = drawer.locator('[data-session-id="session-memory-scope-audit"]');
  await sessionButton.click();

  await expect(drawer).toHaveAttribute("data-open", "false");
  await expect(drawer).toBeHidden();
  await expect(page.getByRole("heading", { level: 1, name: "Memory scope audit" })).toBeFocused();
  await expect(page.locator('[aria-current="page"]')).toHaveAttribute(
    "data-session-id",
    "session-memory-scope-audit",
  );
});
