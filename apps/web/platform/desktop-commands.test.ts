import { afterEach, describe, expect, it, vi } from "vitest";

import {
  desktopProviderCredentialPromptAvailable,
  desktopWorkspacePickerAvailable,
  probeDesktopProvider,
  promptDesktopProviderCredential,
  selectDesktopWorkspace,
  useDesktopProvider,
} from "./desktop-commands";

function installDesktopTransport(): void {
  vi.stubGlobal("window", {
    __ROVE_API_URL__: "http://127.0.0.1:49152",
    __ROVE_TOKEN__: "desktop-transport-token",
  });
}

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
    installDesktopTransport();
    const invoke = vi.fn().mockResolvedValue("D:\\Study\\project\\rove");

    await expect(selectDesktopWorkspace(invoke)).resolves.toBe(
      "D:\\Study\\project\\rove",
    );
    expect(invoke).toHaveBeenCalledWith("workspace_select");
  });

  it("rejects malformed native command results", async () => {
    installDesktopTransport();

    await expect(selectDesktopWorkspace(vi.fn().mockResolvedValue("  "))).rejects.toThrow(
      /invalid workspace path/i,
    );
  });

  it("onboards through safe metadata and returns only a shared Catalog receipt", async () => {
    installDesktopTransport();
    const invoke = vi.fn().mockResolvedValue({
      profile_id: "siliconflow-deepseek-v3-2",
      label: "SiliconFlow DeepSeek V3.2",
      provider_type: "openai",
      api_base: "https://api.siliconflow.cn/v1",
      model: "deepseek-ai/DeepSeek-V3.2",
      catalog_revision: "sha256:catalog-revision",
      credential_source: "keyring",
      probe: {
        inventory_count: 42,
        streaming_supported: true,
        native_tool_calls_supported: true,
        usage_supported: true,
      },
      selected: true,
    });

    expect(desktopProviderCredentialPromptAvailable()).toBe(true);
    await expect(
      promptDesktopProviderCredential(
        {
          profileId: "siliconflow-deepseek-v3-2",
          label: "SiliconFlow DeepSeek V3.2",
          providerType: "openai",
          apiBase: "https://api.siliconflow.cn/v1",
          model: "deepseek-ai/DeepSeek-V3.2",
          expectedRevision: "sha256:previous",
        },
        invoke,
      ),
    ).resolves.toEqual({
      profileId: "siliconflow-deepseek-v3-2",
      label: "SiliconFlow DeepSeek V3.2",
      providerType: "openai",
      apiBase: "https://api.siliconflow.cn/v1",
      model: "deepseek-ai/DeepSeek-V3.2",
      catalogRevision: "sha256:catalog-revision",
      credentialSource: "keyring",
      probe: {
        inventoryCount: 42,
        streamingSupported: true,
        nativeToolCallsSupported: true,
        usageSupported: true,
      },
      selected: true,
    });
    expect(invoke).toHaveBeenCalledWith("provider_credential_prompt", {
      request: {
        profile_id: "siliconflow-deepseek-v3-2",
        label: "SiliconFlow DeepSeek V3.2",
        provider_type: "openai",
        api_base: "https://api.siliconflow.cn/v1",
        model: "deepseek-ai/DeepSeek-V3.2",
        make_default: true,
        expected_revision: "sha256:previous",
      },
    });
    expect(JSON.stringify(invoke.mock.calls)).not.toMatch(
      /api[_-]?key|password|authorization|bearer\s/i,
    );
  });

  it("probes and selects a published profile without a credential payload", async () => {
    installDesktopTransport();
    const probeInvoke = vi.fn().mockResolvedValue({
      inventory_count: 17,
      streaming_supported: true,
      native_tool_calls_supported: true,
      usage_supported: true,
    });
    await expect(
      probeDesktopProvider(
        {
          profileId: "siliconflow-deepseek-v3-2",
          model: "deepseek-ai/DeepSeek-V3.2",
        },
        probeInvoke,
      ),
    ).resolves.toMatchObject({ inventoryCount: 17, nativeToolCallsSupported: true });
    expect(probeInvoke).toHaveBeenCalledWith("provider_profile_probe", {
      request: {
        profile_id: "siliconflow-deepseek-v3-2",
        model: "deepseek-ai/DeepSeek-V3.2",
      },
    });

    const useInvoke = vi.fn().mockResolvedValue({
      profile_id: "siliconflow-deepseek-v3-2",
      model: "deepseek-ai/DeepSeek-V3.2",
      catalog_revision: "sha256:next",
    });
    await expect(
      useDesktopProvider(
        {
          profileId: "siliconflow-deepseek-v3-2",
          model: "deepseek-ai/DeepSeek-V3.2",
          expectedRevision: "sha256:current",
        },
        useInvoke,
      ),
    ).resolves.toEqual({
      profileId: "siliconflow-deepseek-v3-2",
      model: "deepseek-ai/DeepSeek-V3.2",
      catalogRevision: "sha256:next",
    });
    expect(JSON.stringify([probeInvoke.mock.calls, useInvoke.mock.calls])).not.toMatch(
      /api[_-]?key|password|secret|authorization/i,
    );
  });

  it("rejects native Provider commands outside Desktop", async () => {
    const invoke = vi.fn();
    await expect(
      promptDesktopProviderCredential(
        {
          label: "Provider",
          providerType: "openai",
          apiBase: "https://example.test/v1",
          model: "model",
        },
        invoke,
      ),
    ).rejects.toThrow(/requires the Rove Desktop host/i);
    await expect(
      probeDesktopProvider({ profileId: "profile" }, invoke),
    ).rejects.toThrow(/requires the Rove Desktop host/i);
    expect(invoke).not.toHaveBeenCalled();
  });
});
