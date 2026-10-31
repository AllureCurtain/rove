// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CopyProvider } from "../copy/CopyProvider";
import type { ProductTrustStatus } from "./settings-platform-api-types";
import type { SettingsPlatformClient } from "./settings-platform-client";
import { ProviderTrustPrompt } from "./ProviderTrustPrompt";

/**
 * The provider-credentials trust prompt.
 *
 * A turn with a non-fake provider is refused `project_trust_required` until the
 * workspace holds the capability, and nothing else in the UI said so. This is
 * the card that says it: it appears only while the grant is genuinely missing,
 * and its one click grants exactly `provider_credentials` on the workspace the
 * turn would run in. Mounted in jsdom because the trust read is an effect —
 * what renders before and after it is the contract.
 */

function trustStatus(
  granted: readonly string[],
  overrides: Partial<ProductTrustStatus> = {},
): ProductTrustStatus {
  return {
    workspace_id: "workspace-1",
    state: granted.length > 0 ? "trusted" : "restricted",
    granted_capabilities: [...granted],
    invalidated_capabilities: [],
    identity_digest: "sha256:" + "0".repeat(64),
    ...overrides,
  };
}

function clientWith(
  status: ProductTrustStatus,
  decide = vi.fn(async () => trustStatus(["provider_credentials"])),
): { client: SettingsPlatformClient; decide: typeof decide } {
  const getProjectTrust = vi.fn(async () => status);
  const client = {
    getProjectTrust,
    decideProjectTrust: decide,
  } as unknown as SettingsPlatformClient;
  return { client, decide };
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT =
    true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => {
    root.unmount();
  });
  container.remove();
});

async function render(props: {
  client: SettingsPlatformClient;
  workspaceId: string | null;
  providerIsRemote: boolean;
  workspaceName?: string;
}) {
  await act(async () => {
    root.render(
      <CopyProvider>
        <ProviderTrustPrompt {...props} />
      </CopyProvider>,
    );
  });
}

describe("ProviderTrustPrompt", () => {
  it("offers the grant when the workspace lacks provider_credentials", async () => {
    const { client, decide } = clientWith(trustStatus([]));
    await render({
      client,
      workspaceId: "workspace-1",
      providerIsRemote: true,
      workspaceName: "my project",
    });

    const button = container.querySelector<HTMLButtonElement>(
      ".field-actions button",
    );
    expect(button?.textContent).toContain("授权");
    expect(container.textContent).toContain("my project");
    expect(container.textContent).toContain("提供商凭据");

    await act(async () => {
      button?.click();
    });
    expect(decide).toHaveBeenCalledWith("workspace-1", {
      decision: "grant",
      capabilities: ["provider_credentials"],
    });
    // The granted state confirms instead of silently disappearing: the reader
    // pressed something and the card says what the server committed.
    expect(container.textContent).toContain("已授权");
  });

  it("renders nothing while the grant already covers the provider", async () => {
    const { client } = clientWith(trustStatus(["provider_credentials"]));
    await render({
      client,
      workspaceId: "workspace-1",
      providerIsRemote: true,
    });
    expect(container.querySelector(".settings-card")).toBeNull();
  });

  it("renders nothing for a local provider or no workspace", async () => {
    const getProjectTrust = vi.fn(async () => trustStatus([]));
    const client = {
      getProjectTrust,
    } as unknown as SettingsPlatformClient;

    // The local demo profile never hits the gate: no read, no prompt.
    await render({
      client,
      workspaceId: "workspace-1",
      providerIsRemote: false,
    });
    expect(container.querySelector(".settings-card")).toBeNull();

    await render({
      client,
      workspaceId: null,
      providerIsRemote: true,
    });
    expect(container.querySelector(".settings-card")).toBeNull();
    expect(getProjectTrust).not.toHaveBeenCalled();
  });
});
