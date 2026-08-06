import { describe, expect, it } from "vitest";

import {
  ProviderSelectionError,
  assertProviderSelectionIsSatisfiable,
  buildTurnJobRequest,
  isHardResumeError,
} from "./turn-request";
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

const profile: ProviderProfileRecord = {
  id: "profile_1",
  label: "Local",
  providerType: "openai",
  apiBase: "http://127.0.0.1:11434/v1",
  updatedAt: "2026-07-25T00:00:00.000Z",
};

function selectionOf(patch: Partial<ActiveProviderSelection>): ActiveProviderSelection {
  return {
    mode: "profile",
    profileId: profile.id,
    model: "gpt-test",
    approval: "ask",
    maxSteps: 8,
    ...patch,
  };
}

describe("assertProviderSelectionIsSatisfiable", () => {
  it("accepts a selection whose profile is still in the catalog", () => {
    expect(() =>
      assertProviderSelectionIsSatisfiable(selectionOf({}), [profile]),
    ).not.toThrow();
  });

  it("rejects a selection whose profile was removed", () => {
    expect(() =>
      assertProviderSelectionIsSatisfiable(
        selectionOf({ profileId: "profile_missing" }),
        [profile],
      ),
    ).toThrow(ProviderSelectionError);
    expect(() =>
      assertProviderSelectionIsSatisfiable(
        selectionOf({ profileId: "profile_missing" }),
        [profile],
      ),
    ).toThrow(/no longer available/i);
  });

  it("rejects profile mode without a profile id", () => {
    expect(() =>
      assertProviderSelectionIsSatisfiable(
        selectionOf({ profileId: undefined }),
        [profile],
      ),
    ).toThrow(/missing/i);
  });

  it("allows default mode regardless of the catalog", () => {
    expect(() =>
      assertProviderSelectionIsSatisfiable(
        selectionOf({ mode: "default", profileId: undefined }),
        [],
      ),
    ).not.toThrow();
    expect(() => assertProviderSelectionIsSatisfiable(null, [])).not.toThrow();
  });
});

describe("buildTurnJobRequest", () => {
  it("binds the exact product session and omits client resume", () => {
    const request = buildTurnJobRequest({
      message: "hello",
      workspace,
      session: baseSession,
    });

    expect(request).toEqual({
      message: "hello",
      workspace: {
        kind: "folder",
        root: workspace.rootPath,
      },
      product_session_id: "sess_1",
    });
    expect(request).not.toHaveProperty("resume");
  });

  it("keeps exact product binding after restore without adding resume", () => {
    const request = buildTurnJobRequest({
      message: "continue",
      workspace,
      session: { ...baseSession, hasDurableTurn: true },
    });

    expect(request.product_session_id).toBe("sess_1");
    expect(request).not.toHaveProperty("resume");
    expect(request.workspace).toEqual({
      kind: "folder",
      root: workspace.rootPath,
    });
  });

  it("does not send client-owned model, provider, approval, or step fields", () => {
    const request = buildTurnJobRequest({
      message: "server-owned settings",
      workspace,
      session: baseSession,
    });

    expect(request).not.toHaveProperty("model");
    expect(request).not.toHaveProperty("max_steps");
    expect(request).not.toHaveProperty("provider");
    expect(request).not.toHaveProperty("approval");
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
