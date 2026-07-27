import { expect, test, type Page } from "@playwright/test";

import {
  M1_BROWSER_MIGRATION_STATE_KEY,
  M1_BROWSER_STORAGE_KEYS,
} from "../../product/m1-storage-keys";
import { installMockProductApi } from "./product-api-mock";

const LEGACY_WORKSPACE_ROOT = "D:/tmp/rove-migration-e2e";

test("imports M1 browser state before catalog boot and does not replay after refresh", async ({
  page,
}) => {
  const api = await installMockProductApi(page);
  await seedLegacyState(page);

  await page.goto("/");

  await expect(page).toHaveURL(/\/w\/workspace-1\/s\/session-1$/u);
  await expect(page.getByRole("heading", { name: "Imported session" })).toBeVisible();
  await expect(page.getByText("Browser data imported (3 records).")).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  expect(api.migrationRequestBodies).toHaveLength(1);
  expect(api.initialStateReadRequests).toBeGreaterThanOrEqual(3);
  expect(api.workspaces[0]?.canonical_root).toBe(LEGACY_WORKSPACE_ROOT);
  expect(api.providerProfiles[0]).toMatchObject({
    label: "Imported gateway",
    api_key_env: "GATEWAY_API_KEY",
  });
  expect(api.migrationRequestBodies[0]).not.toMatch(/sk-browser-secret|apiKey/iu);

  const migrationState = await page.evaluate((key) => localStorage.getItem(key), M1_BROWSER_MIGRATION_STATE_KEY);
  expect(JSON.parse(migrationState ?? "null")).toMatchObject({ status: "complete" });

  await page.reload();
  await expect(page.getByRole("heading", { name: "Imported session" })).toBeVisible();
  expect(api.migrationRequestBodies).toHaveLength(1);
  await expect(page.getByText(/Browser data imported/iu)).toHaveCount(0);
});

test("keeps the exact pending payload and verifies it before catalog boot", async ({ page }) => {
  const api = await installMockProductApi(page, { migrationFailures: 1 });
  await seedLegacyState(page);

  await page.goto("/");

  await expect(page.getByRole("heading", { name: "Import needs verification" })).toBeVisible();
  expect(api.initialStateReadRequests).toBe(0);
  expect(api.migrationRequestBodies).toHaveLength(1);
  const firstBody = api.migrationRequestBodies[0];
  await expect.poll(() => browserValue(page, M1_BROWSER_STORAGE_KEYS.workspaces)).not.toBeNull();

  await page.getByRole("button", { name: "Verify import" }).click();

  await expect(page.getByRole("heading", { name: "Imported session" })).toBeVisible();
  expect(api.migrationRequestBodies).toHaveLength(2);
  expect(api.migrationRequestBodies[1]).toBe(firstBody);
  expect(api.initialStateReadRequests).toBeGreaterThanOrEqual(3);
});

test("malformed legacy state fails closed and preserves the source key", async ({ page }) => {
  const api = await installMockProductApi(page);
  await page.addInitScript(
    ({ key, value }) => localStorage.setItem(key, value),
    { key: M1_BROWSER_STORAGE_KEYS.workspaces, value: "{not-json" },
  );

  await page.goto("/");

  await expect(page.getByRole("heading", { name: "Saved browser data needs repair" })).toBeVisible();
  await expect(page.getByText(/has not deleted your browser-saved/iu)).toBeVisible();
  expect(api.migrationRequestBodies).toHaveLength(0);
  expect(api.initialStateReadRequests).toBe(0);
  expect(await browserValue(page, M1_BROWSER_STORAGE_KEYS.workspaces)).toBe("{not-json");

  await page.getByRole("button", { name: "Check again" }).click();
  await expect(page.getByRole("heading", { name: "Saved browser data needs repair" })).toBeVisible();
  expect(api.migrationRequestBodies).toHaveLength(0);
  expect(api.initialStateReadRequests).toBe(0);
});

test("rewrites a legacy deep route and carries one migration warning across the reload", async ({
  page,
}) => {
  const api = await installMockProductApi(page, {
    migrationIssues: [
      {
        code: "preference_write_conflict",
        entity: "preferences",
      },
    ],
  });
  await seedLegacyState(page);

  await page.goto(
    "/w/legacy-workspace/s/legacy-session?inspector=open#latest",
  );

  await expect(page).toHaveURL(
    /\/w\/workspace-1\/s\/session-1\?inspector=open#latest$/u,
  );
  await expect(page.getByRole("heading", { name: "Imported session" })).toBeVisible();
  await expect(
    page.getByText(/1 item needs review: Newer server preferences were preserved\./u),
  ).toBeVisible();
  expect(api.migrationRequestBodies).toHaveLength(1);

  await page.reload();
  await expect(page.getByRole("heading", { name: "Imported session" })).toBeVisible();
  await expect(page.getByText(/item needs review/iu)).toHaveCount(0);
  expect(api.migrationRequestBodies).toHaveLength(1);
});

async function seedLegacyState(page: Page) {
  const values: Record<string, string> = {
    [M1_BROWSER_STORAGE_KEYS.workspaces]: JSON.stringify([
      {
        id: "legacy-workspace",
        rootPath: LEGACY_WORKSPACE_ROOT,
        kind: "folder",
        displayName: "Imported workspace",
        pinned: true,
        lastOpenedAt: "2026-07-26T08:00:00.000Z",
      },
    ]),
    [M1_BROWSER_STORAGE_KEYS.sessions]: JSON.stringify([
      {
        id: "legacy-session",
        workspaceId: "legacy-workspace",
        title: "Imported session",
        createdAt: "2026-07-26T08:00:00.000Z",
        updatedAt: "2026-07-26T09:00:00.000Z",
        status: "idle",
        hasDurableTurn: false,
      },
    ]),
    [M1_BROWSER_STORAGE_KEYS.active]: JSON.stringify({
      workspaceId: "legacy-workspace",
      sessionId: "legacy-session",
    }),
    [M1_BROWSER_STORAGE_KEYS.providerProfiles]: JSON.stringify([
      {
        id: "legacy-profile",
        label: "Imported gateway",
        providerType: "openai",
        apiBase: "https://gateway.example.test/v1",
        apiKeyEnv: "GATEWAY_API_KEY",
        apiKey: "sk-browser-secret",
        defaultModel: "gateway/model",
        updatedAt: "2026-07-26T09:00:00.000Z",
      },
    ]),
    [M1_BROWSER_STORAGE_KEYS.providerSelection]: JSON.stringify({
      mode: "profile",
      profileId: "legacy-profile",
      model: "gateway/model",
      approval: "ask",
      maxSteps: 8,
    }),
    [M1_BROWSER_STORAGE_KEYS.theme]: "dark",
  };
  await page.addInitScript((entries) => {
    for (const [key, value] of Object.entries(entries)) {
      localStorage.setItem(key, value);
    }
  }, values);
}

async function browserValue(page: Page, key: string): Promise<string | null> {
  return page.evaluate((storageKey) => localStorage.getItem(storageKey), key);
}
