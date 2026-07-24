import { afterEach, describe, expect, it, vi } from "vitest";

import {
  createJob,
  fetchRunReport,
  listProviderModels,
  listRuns,
  submitApproval,
  testProvider,
} from "./rove-client";

describe("rove client", () => {
  afterEach(() => {
    vi.restoreAllMocks();
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
        channel: "openai",
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
          channel: "openai",
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
        channel: "anthropic",
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
          channel: "anthropic",
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
        channel: "openai",
        wire_protocol: "openai-chat",
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
        channel: "openai",
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
          channel: "openai",
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
        channel: "openai",
        wire_protocol: "openai-chat",
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
        channel: "openai",
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
          channel: "openai",
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
