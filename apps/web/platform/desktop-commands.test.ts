import { afterEach, describe, expect, it, vi } from "vitest";

import {
  desktopWorkspacePickerAvailable,
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
});
