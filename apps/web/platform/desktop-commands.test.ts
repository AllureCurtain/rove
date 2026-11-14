import { afterEach, describe, expect, it, vi } from "vitest";

import {
  desktopCloseActionAvailable,
  desktopExternalLinkOpenerAvailable,
  desktopProviderCredentialPromptAvailable,
  desktopWorkspacePickerAvailable,
  getDesktopCloseAction,
  openDesktopExternalLink,
  probeDesktopProvider,
  promptDesktopProviderCredential,
  selectDesktopWorkspace,
  setDesktopCloseAction,
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
    const invoke = vi.fn().mockResolvedValue("C:\\workspace\\demo");

    await expect(selectDesktopWorkspace(invoke)).resolves.toBe(
      "C:\\workspace\\demo",
    );
    expect(invoke).toHaveBeenCalledWith("workspace_select");
  });

  it("does not invoke the Windows credential command for loopback-entry hosts", async () => {
    installDesktopTransport();
    window.__ROVE_CREDENTIAL_ENTRY__ = "loopback";
    expect(desktopWorkspacePickerAvailable()).toBe(true);
    expect(desktopProviderCredentialPromptAvailable()).toBe(false);
    const invoke = vi.fn();
    await expect(promptDesktopProviderCredential({
      label: "Example", providerType: "openai", apiBase: "https://example.test/v1", model: "model",
    }, invoke)).rejects.toThrow();
    expect(invoke).not.toHaveBeenCalled();
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

  it("reads and writes the host close action with strict parsing", async () => {
    installDesktopTransport();

    expect(desktopCloseActionAvailable()).toBe(true);
    const getInvoke = vi.fn().mockResolvedValue("hide_to_tray");
    await expect(getDesktopCloseAction(getInvoke)).resolves.toBe("hide_to_tray");
    expect(getInvoke).toHaveBeenCalledWith("get_close_action");

    const setInvoke = vi.fn().mockResolvedValue(undefined);
    await expect(setDesktopCloseAction("quit", setInvoke)).resolves.toBeUndefined();
    expect(setInvoke).toHaveBeenCalledWith("set_close_action", { action: "quit" });

    await expect(
      getDesktopCloseAction(vi.fn().mockResolvedValue("minimize")),
    ).rejects.toThrow(/invalid close action/i);
  });

  it("rejects close-action commands outside Desktop", async () => {
    const invoke = vi.fn();

    expect(desktopCloseActionAvailable()).toBe(false);
    await expect(getDesktopCloseAction(invoke)).rejects.toThrow(
      /requires the Rove Desktop host/i,
    );
    await expect(setDesktopCloseAction("quit", invoke)).rejects.toThrow(
      /requires the Rove Desktop host/i,
    );
    expect(invoke).not.toHaveBeenCalled();
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

describe("openDesktopExternalLink", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("reports unsupported instead of guessing outside Desktop", async () => {
    const invoke = vi.fn();

    expect(desktopExternalLinkOpenerAvailable()).toBe(false);
    await expect(
      openDesktopExternalLink("https://example.test/docs", invoke),
    ).resolves.toEqual({ status: "unsupported" });
    expect(invoke).not.toHaveBeenCalled();
  });

  it("hands the url to the controlled host command", async () => {
    installDesktopTransport();
    const invoke = vi.fn().mockResolvedValue(undefined);

    await expect(
      openDesktopExternalLink("https://example.test/docs", invoke),
    ).resolves.toEqual({ status: "opened" });
    expect(invoke).toHaveBeenCalledWith("open_external", {
      url: "https://example.test/docs",
    });
  });

  it("separates a host rejection from an OS launch failure", async () => {
    installDesktopTransport();

    await expect(
      openDesktopExternalLink(
        "file:///etc/passwd",
        vi.fn().mockRejectedValue(
          new Error("only absolute http:// and https:// URLs are allowed"),
        ),
      ),
    ).resolves.toEqual({
      status: "blocked",
      detail: "only absolute http:// and https:// URLs are allowed",
    });
    await expect(
      openDesktopExternalLink(
        "https://example.test/docs",
        vi.fn().mockRejectedValue(new Error("failed to open URL: no browser")),
      ),
    ).resolves.toEqual({
      status: "failed",
      detail: "failed to open URL: no browser",
    });
  });
});
