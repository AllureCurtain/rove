import { afterEach, describe, expect, it, vi } from "vitest";

const ORIGINAL_ENV = { ...process.env };

afterEach(() => {
  process.env = { ...ORIGINAL_ENV };
  vi.resetModules();
});

async function loadPlaywrightConfig() {
  vi.resetModules();
  const module = await import("./playwright.config");
  return module.default;
}

function webServer(config: Awaited<ReturnType<typeof loadPlaywrightConfig>>) {
  const server = config.webServer;
  if (Array.isArray(server)) {
    throw new Error("expected a single webServer config");
  }
  if (!server) {
    throw new Error("expected webServer config");
  }
  return server;
}

describe("Playwright config", () => {
  it("uses an isolated default port and starts its own server", async () => {
    delete process.env.PLAYWRIGHT_BASE_URL;
    delete process.env.ROVE_WEB_PORT;
    delete process.env.CI;

    const config = await loadPlaywrightConfig();
    const server = webServer(config);

    expect(config.use?.baseURL).toBe("http://127.0.0.1:13043");
    expect(server.url).toBe("http://127.0.0.1:13043");
    expect(server.command).toContain("--port 13043");
    expect(server.reuseExistingServer).toBe(false);
  });

  it("reuses an explicitly provided base URL", async () => {
    process.env.PLAYWRIGHT_BASE_URL = "http://127.0.0.1:13123";
    process.env.ROVE_WEB_PORT = "13123";

    const config = await loadPlaywrightConfig();
    const server = webServer(config);

    expect(config.use?.baseURL).toBe("http://127.0.0.1:13123");
    expect(server.url).toBe("http://127.0.0.1:13123");
    expect(server.command).toContain("--port 13123");
    expect(server.reuseExistingServer).toBe(true);
  });
});
