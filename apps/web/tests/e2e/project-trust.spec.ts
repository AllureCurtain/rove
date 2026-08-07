import { expect, test } from "@playwright/test";

import { createMockWorkspace, installMockProductApi } from "./product-api-mock";

test("project trust exposes every state, capability decisions, and digest invalidation", async ({
  page,
}) => {
  const workspace = createMockWorkspace(
    "workspace-trust",
    "D:/private/operator/workspace-trust",
  );
  const api = await installMockProductApi(page, {
    workspaces: [workspace],
    activeWorkspaceId: workspace.id,
  });
  await page.goto("/settings/workspace");

  const trustCard = page.locator(".settings-card").filter({
    has: page.getByRole("heading", { name: "Project trust" }),
  });
  await expect(trustCard).toContainText("Unknown");

  const capabilityInputs = trustCard.locator(".trust-capabilities input");
  await expect(capabilityInputs).toHaveCount(6);
  for (const index of [2, 3, 4, 5]) {
    await capabilityInputs.nth(index).uncheck();
  }
  await trustCard.getByRole("button", { name: "Grant selected" }).click();
  await expect(trustCard).toContainText("Trusted");
  await expect.poll(() => api.trustStatuses[workspace.id]?.granted_capabilities).toEqual([
    "project_configuration",
    "workspace_instructions",
  ]);

  const grantRequest = api.trustRequests.find(
    (request) => request.method === "PUT" && request.body?.includes('"grant"'),
  );
  expect(grantRequest?.body).toContain("project_configuration");
  expect(grantRequest?.body).toContain("workspace_instructions");
  expect(grantRequest?.body).not.toContain(workspace.canonical_root);
  for (const request of api.trustRequests) {
    expect(request.workspaceId).toBe(workspace.id);
  }

  api.trustStatuses[workspace.id] = {
    ...api.trustStatuses[workspace.id]!,
    state: "trusted",
    invalidated_capabilities: ["workspace_instructions"],
    granted_capabilities: ["project_configuration"],
  };
  await trustCard.getByRole("button", { name: "Refresh project trust" }).click();
  await expect(trustCard).toContainText("Changed");
  await expect(trustCard).toContainText("Trusted");

  await trustCard.getByRole("button", { name: "Deny" }).click();
  await expect(trustCard).toContainText("Restricted");
  await trustCard.getByRole("button", { name: "Grant selected" }).click();
  await expect(trustCard).toContainText("Trusted");
  await trustCard.getByRole("button", { name: "Revoke" }).click();
  await expect(trustCard).toContainText("Revoked");

  expect(api.trustRequests.filter((request) => request.method === "PUT")).toHaveLength(4);
});
