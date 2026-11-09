import { afterEach, describe, expect, it, vi } from "vitest";

import {
  desktopTransport,
  desktopCredentialEntry,
  providerCredentialEntryAvailable,
  withDesktopAuthorization,
} from "./desktop-transport";

describe("desktop transport", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("enables paste only for an explicitly capable authenticated local host", () => {
    const host = {
      location: { protocol: "tauri:", hostname: "localhost" },
      __ROVE_API_URL__: "http://127.0.0.1:49152",
      __ROVE_TOKEN__: "fixture-token",
      __ROVE_CREDENTIAL_ENTRY__: "loopback",
    };
    vi.stubGlobal("window", host);
    expect(desktopCredentialEntry()).toBe("loopback");
    expect(providerCredentialEntryAvailable()).toBe(true);
    for (const override of [
      { __ROVE_API_URL__: "https://remote.example/api" }, { __ROVE_TOKEN__: "" },
      { __ROVE_CREDENTIAL_ENTRY__: "invalid" }, { __ROVE_CREDENTIAL_ENTRY__: undefined },
      { __ROVE_CREDENTIAL_ENTRY__: "native" },
    ]) {
      vi.stubGlobal("window", { ...host, ...override });
      expect(providerCredentialEntryAvailable()).toBe(false);
    }
  });

  it("keeps legacy hosts on native entry and ordinary remote browsers disabled", () => {
    vi.stubGlobal("window", { __ROVE_API_URL__: "http://127.0.0.1:49152", __ROVE_TOKEN__: "fixture-token" });
    expect(desktopCredentialEntry()).toBe("native");
    for (const [protocol, hostname, allowed] of [
      ["http:", "localhost", true], ["https:", "127.0.0.1", true],
      ["http:", "[::1]", true], ["https:", "remote.example", false], ["tauri:", "localhost", false],
    ] as const) {
      vi.stubGlobal("window", { location: { protocol, hostname } });
      expect(providerCredentialEntryAvailable()).toBe(allowed);
    }
  });

  it("accepts only an injected loopback HTTP API with a token", () => {
    vi.stubGlobal("window", {
      __ROVE_API_URL__: "http://127.0.0.1:49152",
      __ROVE_TOKEN__: "desktop-secret",
    });
    expect(desktopTransport()).toEqual({
      apiPrefix: "http://127.0.0.1:49152",
      token: "desktop-secret",
    });

    vi.stubGlobal("window", {
      __ROVE_API_URL__: "https://remote.example/api",
      __ROVE_TOKEN__: "desktop-secret",
    });
    expect(desktopTransport()).toBeNull();
  });

  it("adds bearer authorization without dropping request headers", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
    const authorized = withDesktopAuthorization(fetchMock, "desktop-secret");
    await authorized("http://127.0.0.1:49152/product/runtime", {
      headers: { accept: "application/json" },
    });
    const headers = new Headers(fetchMock.mock.calls[0]?.[1]?.headers);
    expect(headers.get("accept")).toBe("application/json");
    expect(headers.get("authorization")).toBe("Bearer desktop-secret");
  });
});
