import { afterEach, describe, expect, it, vi } from "vitest";

import {
  desktopTransport,
  withDesktopAuthorization,
} from "./desktop-transport";

describe("desktop transport", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
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
