import { describe, expect, it, vi } from "vitest";

import { proxyRoveApiRequest } from "./rove-api-proxy";

describe("rove API proxy", () => {
  it("injects the server-side API token into upstream requests", async () => {
    const fetchImpl = vi.fn().mockResolvedValue(new Response("{}", { status: 200 }));
    const request = new Request("http://localhost:3000/api/jobs?trace=1", {
      method: "POST",
      headers: {
        "content-type": "application/json",
      },
      body: JSON.stringify({ message: "hello" }),
    });

    await proxyRoveApiRequest(request, ["jobs"], {
      apiBase: "http://127.0.0.1:8787",
      apiToken: "secret-token",
      fetchImpl,
    });

    expect(fetchImpl).toHaveBeenCalledTimes(1);
    const [url, init] = fetchImpl.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("http://127.0.0.1:8787/jobs?trace=1");
    expect(new Headers(init.headers).get("authorization")).toBe(
      "Bearer secret-token",
    );
    expect(new Headers(init.headers).get("content-type")).toBe("application/json");
  });

  it("omits authorization when no server-side token is configured", async () => {
    const fetchImpl = vi.fn().mockResolvedValue(new Response("{}", { status: 200 }));
    const request = new Request("http://localhost:3000/api/jobs/job-1/state");

    await proxyRoveApiRequest(request, ["jobs", "job-1", "state"], {
      apiBase: "http://127.0.0.1:8787",
      apiToken: "",
      fetchImpl,
    });

    const [, init] = fetchImpl.mock.calls[0] as [string, RequestInit];
    expect(new Headers(init.headers).has("authorization")).toBe(false);
  });

  it("does not forward browser Origin/Referer to the Rust API", async () => {
    const fetchImpl = vi.fn().mockResolvedValue(new Response("{}", { status: 200 }));
    const request = new Request("http://localhost:3000/api/bench/runs", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        origin: "http://localhost:3000",
        referer: "http://localhost:3000/",
      },
      body: JSON.stringify({ suite: "dataprep", profile: "default" }),
    });

    await proxyRoveApiRequest(request, ["bench", "runs"], {
      apiBase: "http://127.0.0.1:8787",
      fetchImpl,
    });

    const [url, init] = fetchImpl.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("http://127.0.0.1:8787/bench/runs");
    const headers = new Headers(init.headers);
    expect(headers.has("origin")).toBe(false);
    expect(headers.has("referer")).toBe(false);
    expect(headers.get("content-type")).toBe("application/json");
  });

  it("passes through SSE response bodies and streaming headers", async () => {
    const stream = new ReadableStream({
      start(controller) {
        controller.enqueue(new TextEncoder().encode("event: message\n\n"));
        controller.close();
      },
    });
    const upstream = new Response(stream, {
      status: 200,
      headers: {
        "content-type": "text/event-stream",
        "cache-control": "no-cache",
      },
    });
    const fetchImpl = vi.fn().mockResolvedValue(upstream);
    const request = new Request("http://localhost:3000/api/jobs/job-1/events");

    const response = await proxyRoveApiRequest(
      request,
      ["jobs", "job-1", "events"],
      {
        apiBase: "http://127.0.0.1:8787",
        fetchImpl,
      },
    );

    expect(response.headers.get("content-type")).toContain("text/event-stream");
    expect(response.headers.get("cache-control")).toBe("no-cache");
    await expect(response.text()).resolves.toBe("event: message\n\n");
  });
});
