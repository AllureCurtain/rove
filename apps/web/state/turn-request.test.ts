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
  it("binds the exact product session and omits client resume", () => {
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
      workspace: {
        kind: "folder",
        root: "D:\\\\Study\\\\project",
      },
      provider: undefined,
      product_session_id: "sess_1",
    });
    expect(request).not.toHaveProperty("resume");
  });

  it("keeps exact product binding after restore without adding resume", () => {
    const request = buildTurnJobRequest({
      message: "continue",
      workspace,
      session: { ...baseSession, hasDurableTurn: true },
      selection,
      profiles: [],
    });

    expect(request.product_session_id).toBe("sess_1");
    expect(request).not.toHaveProperty("resume");
    expect(request.workspace).toEqual({
      kind: "folder",
      root: "D:\\\\Study\\\\project",
    });
  });

  it("defers approval to the server preference when no selection is explicit", () => {
    const request = buildTurnJobRequest({
      message: "use the durable default",
      workspace,
      session: baseSession,
      selection: { ...selection, approval: "never" },
      profiles: [],
      useDefaultApproval: true,
    });

    expect(request).not.toHaveProperty("approval");
  });

  it("sends approval when the provider selection is explicit", () => {
    const request = buildTurnJobRequest({
      message: "use the explicit selection",
      workspace,
      session: baseSession,
      selection: { ...selection, approval: "auto" },
      profiles: [],
      useDefaultApproval: false,
    });

    expect(request.approval).toBe("auto");
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
    expect(request.product_session_id).toBe("sess_1");
    expect(request).not.toHaveProperty("resume");
    expect(JSON.stringify(request)).not.toMatch(/sk-/);
  });

  it("fails closed when the selected provider profile is unavailable", () => {
    expect(() =>
      buildTurnJobRequest({
        message: "hi",
        workspace,
        session: baseSession,
        selection: {
          mode: "profile",
          profileId: "prov_missing",
          model: "gpt-test",
          approval: "ask",
          maxSteps: 4,
        },
        profiles: [],
      }),
    ).toThrow(/no longer available/i);
  });
});

describe("isHardResumeError", () => {
  it("detects fail-closed resume failures", () => {
    expect(isHardResumeError("no durable task_state in workspace")).toBe(true);
    expect(isHardResumeError("resume failed: not resumable")).toBe(true);
    expect(isHardResumeError("product_session_runtime_state_missing")).toBe(true);
    expect(isHardResumeError("network down")).toBe(false);
  });
});
