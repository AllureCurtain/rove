import { describe, expect, it } from "vitest";

import { buildTurnJobRequest, isHardResumeError } from "./turn-request";
import type {
  ActiveProviderSelection,
  ProviderProfileRecord,
  SessionRecord,
  WorkspaceRecord,
} from "./product-types";

const workspace: WorkspaceRecord = {
  id: "ws_1",
  rootPath: "D:\\\\Study\\\\project",
  kind: "folder",
  displayName: "project",
  pinned: false,
  lastOpenedAt: "2026-07-25T00:00:00.000Z",
};

const baseSession: SessionRecord = {
  id: "sess_1",
  workspaceId: "ws_1",
  title: "New session",
  createdAt: "2026-07-25T00:00:00.000Z",
  updatedAt: "2026-07-25T00:00:00.000Z",
  status: "idle",
  hasDurableTurn: false,
};

const selection: ActiveProviderSelection = {
  mode: "default",
  model: "fake",
  approval: "ask",
  maxSteps: 8,
};

describe("buildTurnJobRequest", () => {
  it("binds folder root and omits resume on first durable turn", () => {
    const request = buildTurnJobRequest({
      message: "hello",
      workspace,
      session: baseSession,
      selection,
      profiles: [],
    });

    expect(request).toEqual({
      message: "hello",
      model: "fake",
      max_steps: 8,
      approval: "ask",
      resume: undefined,
      workspace: {
        kind: "folder",
        root: "D:\\\\Study\\\\project",
      },
      provider: undefined,
    });
  });

  it("forces hard resume latest after a durable turn", () => {
    const request = buildTurnJobRequest({
      message: "continue",
      workspace,
      session: { ...baseSession, hasDurableTurn: true },
      selection,
      profiles: [],
    });

    expect(request.resume).toBe("latest");
    expect(request.workspace).toEqual({
      kind: "folder",
      root: "D:\\\\Study\\\\project",
    });
  });

  it("injects saved provider profile without raw keys", () => {
    const profiles: ProviderProfileRecord[] = [
      {
        id: "prov_1",
        label: "Relay",
        providerType: "openai",
        apiBase: "https://relay.example/v1",
        apiKeyEnv: "OPENAI_API_KEY",
        defaultModel: "gpt-test",
        updatedAt: "2026-07-25T00:00:00.000Z",
      },
    ];
    const request = buildTurnJobRequest({
      message: "hi",
      workspace: { ...workspace, kind: "repo" },
      session: baseSession,
      selection: {
        mode: "profile",
        profileId: "prov_1",
        model: "gpt-test",
        approval: "ask",
        maxSteps: 4,
      },
      profiles,
    });

    expect(request.workspace).toEqual({
      kind: "repo",
      root: "D:\\\\Study\\\\project",
    });
    expect(request.provider).toEqual({
      provider_type: "openai",
      name: "Relay",
      api_base: "https://relay.example/v1",
      api_key_env: "OPENAI_API_KEY",
    });
    expect(JSON.stringify(request)).not.toMatch(/sk-/);
  });
});

describe("isHardResumeError", () => {
  it("detects fail-closed resume failures", () => {
    expect(isHardResumeError("no durable task_state in workspace")).toBe(true);
    expect(isHardResumeError("resume failed: not resumable")).toBe(true);
    expect(isHardResumeError("network down")).toBe(false);
  });
});
