import { afterEach, describe, expect, it, vi } from "vitest";

import {
  desktopProviderCredentialPromptAvailable,
  desktopWorkspacePickerAvailable,
  promptDesktopProviderCredential,
  selectDesktopWorkspace,
} from "./desktop-commands";

describe("Desktop commands", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("stays unavailable in the browser transport", async () => {
    const invoke = vi.fn();

    expect(desktopWorkspacePickerAvailable()).toBe(false);
    await expect(selectDesktopWorkspace(invoke)).resolves.toBeNull();
    expect(invoke).not.toHaveBeenCalled();
  });

  it("invokes the bounded native workspace picker", async () => {
    vi.stubGlobal("window", {
      __ROVE_API_URL__: "http://127.0.0.1:49152",
      __ROVE_TOKEN__: "desktop-secret",
    });
    const invoke = vi.fn().mockResolvedValue("D:\\Study\\project\\rove");

    await expect(selectDesktopWorkspace(invoke)).resolves.toBe(
      "D:\\Study\\project\\rove",
    );
    expect(invoke).toHaveBeenCalledWith("workspace_select");
  });

  it("rejects malformed native command results", async () => {
    vi.stubGlobal("window", {
      __ROVE_API_URL__: "http://127.0.0.1:49152",
      __ROVE_TOKEN__: "desktop-secret",
    });

    await expect(selectDesktopWorkspace(vi.fn().mockResolvedValue("  "))).rejects.toThrow(
      /invalid workspace path/i,
    );
  });

  it("keeps raw provider credentials behind the native command boundary", async () => {
    vi.stubGlobal("window", {
      __ROVE_API_URL__: "http://127.0.0.1:49152",
      __ROVE_TOKEN__: "desktop-secret",
    });
    const invoke = vi.fn().mockResolvedValue({
      profile_id: "siliconflow-deepseek-v3-2",
      source: "keyring",
      service: "com.rove.agent.provider",
      account: "profile:siliconflow-deepseek-v3-2:550e8400-e29b-41d4-a716-446655440000",
    });

    expect(desktopProviderCredentialPromptAvailable()).toBe(true);
    await expect(
      promptDesktopProviderCredential(
        {
          profileId: "siliconflow-deepseek-v3-2",
          label: "SiliconFlow DeepSeek V3.2",
        },
        invoke,
      ),
    ).resolves.toEqual({
      profileId: "siliconflow-deepseek-v3-2",
      source: "keyring",
      service: "com.rove.agent.provider",
      account: "profile:siliconflow-deepseek-v3-2:550e8400-e29b-41d4-a716-446655440000",
    });
    expect(invoke).toHaveBeenCalledWith("provider_credential_prompt", {
      request: {
        profile_id: "siliconflow-deepseek-v3-2",
        label: "SiliconFlow DeepSeek V3.2",
      },
    });
    expect(JSON.stringify(invoke.mock.calls)).not.toMatch(/api[_-]?key|password|secret/i);
  });

  it("rejects provider credential prompting outside Desktop", async () => {
    const invoke = vi.fn();
    await expect(
      promptDesktopProviderCredential({ profileId: "profile", label: "Provider" }, invoke),
    ).rejects.toThrow(/requires the Rove Desktop host/i);
    expect(invoke).not.toHaveBeenCalled();
  });
});
