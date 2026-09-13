import { afterEach, describe, expect, it, vi } from "vitest";

import {
  createJob,
  fetchRunReport,
  listProviderModels,
  listRuns,
  openJobStream,
  RoveApiError,
  submitApproval,
  testProvider,
} from "./rove-client";

describe("rove client", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("uses the authenticated loopback transport injected by Desktop", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ job_id: "job-1", run_id: "run-1" }),
    });
    vi.stubGlobal("fetch", fetchMock);
    vi.stubGlobal("window", {
      __ROVE_API_URL__: "http://127.0.0.1:49152",
      __ROVE_TOKEN__: "desktop-secret",
    });

    await createJob({
      message: "desktop run",
      model: "fake",
      max_steps: 1,
      approval: "ask",
    });

    expect(fetchMock.mock.calls[0]?.[0]).toBe("http://127.0.0.1:49152/jobs");
    const headers = new Headers(fetchMock.mock.calls[0]?.[1]?.headers);
    expect(headers.get("authorization")).toBe("Bearer desktop-secret");
    expect(headers.get("content-type")).toBe("application/json");
  });

  it("parses authenticated Desktop SSE frames and exposes canonical event ids", async () => {
    const encoder = new TextEncoder();
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(encoder.encode(": keep-alive\r\n"));
        controller.enqueue(
          encoder.encode(
            "id: 7\r\nevent: run_completed\r\ndata: {\"type\":\"run_completed\"}\r\n\r\n",
          ),
        );
        controller.close();
      },
    });
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      body,
    });
    vi.stubGlobal("fetch", fetchMock);
    vi.stubGlobal("window", {
      __ROVE_API_URL__: "http://127.0.0.1:49152",
      __ROVE_TOKEN__: "desktop-secret",
    });

    const source = openJobStream("job/1");
    const message = await new Promise<MessageEvent<string>>((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error("Desktop SSE event timed out")), 1_000);
      source.addEventListener("run_completed", ((event: MessageEvent<string>) => {
        clearTimeout(timeout);
        source.close();
        resolve(event);
      }) as EventListener);
    });

    expect(message.data).toBe('{"type":"run_completed"}');
    expect(message.lastEventId).toBe("7");
    expect(fetchMock.mock.calls[0]?.[0]?.toString()).toBe(
      "http://127.0.0.1:49152/jobs/job%2F1/events",
    );
    const headers = new Headers(fetchMock.mock.calls[0]?.[1]?.headers);
    expect(headers.get("authorization")).toBe("Bearer desktop-secret");
    expect(headers.get("accept")).toBe("text/event-stream");
  });

  it("reconnects Desktop SSE from the last canonical event without resubmitting", async () => {
    const encoder = new TextEncoder();
    const stream = (frame: string) =>
      new ReadableStream<Uint8Array>({
        start(controller) {
          controller.enqueue(encoder.encode(frame));
          controller.close();
        },
      });
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        body: stream('id: 7\nevent: model_delta\ndata: {"type":"model_delta","delta":"a"}\n\n'),
      })
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        body: stream('id: 8\nevent: run_completed\ndata: {"type":"run_completed"}\n\n'),
      });
    vi.stubGlobal("fetch", fetchMock);
    vi.stubGlobal("window", {
      __ROVE_API_URL__: "http://127.0.0.1:49152",
      __ROVE_TOKEN__: "desktop-secret",
    });

    const source = openJobStream("job-1");
    await new Promise<void>((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error("Desktop SSE reconnect timed out")), 3_000);
      source.addEventListener("run_completed", (() => {
        clearTimeout(timeout);
        source.close();
        resolve();
      }) as EventListener);
    });

    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(fetchMock.mock.calls[1]?.[0]?.toString()).toBe(
      "http://127.0.0.1:49152/jobs/job-1/events?after=7",
    );
    for (const call of fetchMock.mock.calls) {
      const headers = new Headers(call[1]?.headers);
      expect(headers.get("authorization")).toBe("Bearer desktop-secret");
    }
  });

  it("posts approval decisions to the job approval endpoint", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        job_id: "job-1",
        run_id: "run-1",
        status: "running",
        event_count: 4,
        pending_approvals: [],
      }),
    });
    vi.stubGlobal("fetch", fetchMock);

    await submitApproval("job/1", "call/1", "approve");

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/jobs/job%2F1/approvals/call%2F1",
      {
        method: "POST",
        headers: {
          "content-type": "application/json",
        },
        body: JSON.stringify({ decision: "approve" }),
      },
    );
  });

  it("sends approval policy when creating a job", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        job_id: "job-1",
        run_id: "run-1",
      }),
    });
    vi.stubGlobal("fetch", fetchMock);

    await createJob({
      message: "write file",
      model: "fake",
      max_steps: 2,
      approval: "ask",
    });

    expect(fetchMock).toHaveBeenCalledWith("/api/jobs", {
      method: "POST",
      headers: {
        "content-type": "application/json",
      },
      body: JSON.stringify({
        message: "write file",
        model: "fake",
        max_steps: 2,
        approval: "ask",
      }),
    });
  });

  it("sends provider profile when creating a job", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        job_id: "job-1",
        run_id: "run-1",
      }),
    });
    vi.stubGlobal("fetch", fetchMock);

    await createJob({
      message: "run against relay",
      model: "relay/deepseek-v3.2",
      max_steps: 2,
      approval: "ask",
      provider: {
        provider_type: "openai",
        api_base: "https://gateway.test/v1",
        api_key_env: "GATEWAY_API_KEY",
      },
    });

    expect(fetchMock).toHaveBeenCalledWith("/api/jobs", {
      method: "POST",
      headers: {
        "content-type": "application/json",
      },
      body: JSON.stringify({
        message: "run against relay",
        model: "relay/deepseek-v3.2",
        max_steps: 2,
        approval: "ask",
        provider: {
          provider_type: "openai",
          api_base: "https://gateway.test/v1",
          api_key_env: "GATEWAY_API_KEY",
        },
      }),
    });
  });

  it("sends non-OpenAI provider profiles when creating a job", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        job_id: "job-1",
        run_id: "run-1",
      }),
    });
    vi.stubGlobal("fetch", fetchMock);

    await createJob({
      message: "run claude",
      model: "claude-3-5-haiku-latest",
      provider: {
        provider_type: "anthropic",
        api_base: "https://api.anthropic.com",
        api_key_env: "ANTHROPIC_API_KEY",
      },
    });

    expect(fetchMock).toHaveBeenCalledWith("/api/jobs", {
      method: "POST",
      headers: {
        "content-type": "application/json",
      },
      body: JSON.stringify({
        message: "run claude",
        model: "claude-3-5-haiku-latest",
        provider: {
          provider_type: "anthropic",
          api_base: "https://api.anthropic.com",
          api_key_env: "ANTHROPIC_API_KEY",
        },
      }),
    });
  });

  it("tests provider profiles without sending key values", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        status: "pass",
        provider: "gateway.test",
        provider_type: "openai",
        api_base: "https://gateway.test/v1",
        key_env: "GATEWAY_API_KEY",
        key_present: true,
        model: "relay/deepseek-v3.2",
        model_present: true,
        models_count: 8,
      }),
    });
    vi.stubGlobal("fetch", fetchMock);

    const result = await testProvider({
      provider: {
        provider_type: "openai",
        api_base: "https://gateway.test/v1",
        api_key_env: "GATEWAY_API_KEY",
      },
      model: "relay/deepseek-v3.2",
    });

    expect(fetchMock).toHaveBeenCalledWith("/api/providers/test", {
      method: "POST",
      headers: {
        "content-type": "application/json",
      },
      body: JSON.stringify({
        provider: {
          provider_type: "openai",
          api_base: "https://gateway.test/v1",
          api_key_env: "GATEWAY_API_KEY",
        },
        model: "relay/deepseek-v3.2",
      }),
    });
    expect(JSON.stringify(fetchMock.mock.calls)).not.toContain("secret");
    expect(result.model_present).toBe(true);
  });

  it("lists provider models without sending key values", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        provider: "gateway.test",
        provider_type: "openai",
        api_base: "https://gateway.test/v1",
        key_env: "GATEWAY_API_KEY",
        key_present: true,
        models: ["relay/deepseek-v3.2", "official/gpt-compatible"],
        models_count: 2,
      }),
    });
    vi.stubGlobal("fetch", fetchMock);

    const result = await listProviderModels({
      provider: {
        provider_type: "openai",
        api_base: "https://gateway.test/v1",
        api_key_env: "GATEWAY_API_KEY",
      },
    });

    expect(fetchMock).toHaveBeenCalledWith("/api/providers/models", {
      method: "POST",
      headers: {
        "content-type": "application/json",
      },
      body: JSON.stringify({
        provider: {
          provider_type: "openai",
          api_base: "https://gateway.test/v1",
          api_key_env: "GATEWAY_API_KEY",
        },
      }),
    });
    expect(JSON.stringify(fetchMock.mock.calls)).not.toContain("secret");
    expect(result.models).toEqual([
      "relay/deepseek-v3.2",
      "official/gpt-compatible",
    ]);
    expect(result.models_count).toBe(2);
  });

  it("preserves typed provider failure codes without exposing arbitrary bodies", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: false,
      status: 429,
      text: async () => JSON.stringify({
        code: "provider_rate_limited",
        error: "provider rate limited the inventory request",
      }),
    });
    vi.stubGlobal("fetch", fetchMock);

    const request = testProvider({
      provider: {
        provider_type: "fake",
        api_base: "",
      },
    });
    await expect(request).rejects.toMatchObject({
      name: "RoveApiError",
      status: 429,
      code: "provider_rate_limited",
      message: "provider rate limited the inventory request",
    } satisfies Partial<RoveApiError>);
  });

  it("sends resume mode when creating a resumed job", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        job_id: "job-1",
        run_id: "run-2",
        resumed_from_run_id: "run-1",
      }),
    });
    vi.stubGlobal("fetch", fetchMock);

    await createJob({
      message: "continue",
      model: "fake",
      max_steps: 4,
      approval: "ask",
      resume: "latest",
    });

    expect(fetchMock).toHaveBeenCalledWith("/api/jobs", {
      method: "POST",
      headers: {
        "content-type": "application/json",
      },
      body: JSON.stringify({
        message: "continue",
        model: "fake",
        max_steps: 4,
        approval: "ask",
        resume: "latest",
      }),
    });
  });

  it("fetches recent runs", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        runs: [
          {
            run_id: "run-1",
            session_id: "session-1",
            job_id: "job-1",
            status: "done",
            last_event_seq: 5,
            has_report: true,
          },
        ],
      }),
    });
    vi.stubGlobal("fetch", fetchMock);

    const result = await listRuns(25);

    expect(fetchMock).toHaveBeenCalledWith("/api/runs?limit=25");
    expect(result.runs[0].run_id).toBe("run-1");
  });

  it("fetches a run report", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        session_id: "session-1",
        job_id: "job-1",
        run_id: "run-1",
        workspace_root: "D:/workspace",
        workspace_kind: "folder",
        model_id: "fake",
        status: "success",
        termination_reason: "final",
        steps: 1,
        total_usage: { prompt_tokens: 0, completion_tokens: 0, total_tokens: 0 },
        tool_calls: 0,
        tool_failures: 0,
        tool_mutations: [],
        output: "done",
        timestamp: "2026-05-30T00:00:00Z",
      }),
    });
    vi.stubGlobal("fetch", fetchMock);

    const result = await fetchRunReport("run/1");

    expect(fetchMock).toHaveBeenCalledWith("/api/runs/run%2F1/report");
    expect(result.output).toBe("done");
  });
});
