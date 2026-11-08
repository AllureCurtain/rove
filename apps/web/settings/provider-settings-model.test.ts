import { describe, expect, it } from "vitest";

import { createTranslator } from "../copy";
import { describeProviderProbeFailure, providerDraftIssue } from "./provider-settings-model";

describe("provider settings failure reasons", () => {
  it.each([
    ["provider_timeout", "timed out"],
    ["provider_authentication", "authentication failed"],
    ["provider_rate_limited", "rate limit reached"],
    ["provider_protocol_mismatch", "compatible model catalog"],
    ["provider_no_models", "no usable models"],
    ["provider_transport", "could not be reached"],
    ["bad_request", "environment variable"],
    ["provider_onboarding_loopback_required", "local connection"],
  ])("distinguishes %s", (code, expected) => {
    expect(describeProviderProbeFailure({ code, message: "generic" }).toLowerCase())
      .toContain(expected);
  });

  it("never renders untrusted exception bodies in either locale", () => {
    for (const locale of ["en-US", "zh-CN"] as const) {
      const t = createTranslator(locale);
      for (const error of [new Error("private-response-body"), { code: "future_error", message: "private-response-body" }]) {
        expect(describeProviderProbeFailure(error, t)).toBe(t("settings.providers.errors.unknown"));
        expect(describeProviderProbeFailure(error, t)).not.toContain("private-response-body");
      }
    }
  });

  it("validates actionable fields without echoing their contents", () => {
    const input = { label: "Example", providerType: "openai" as const, apiBase: "https://example.test/v1", defaultModel: "model" };
    for (const apiBase of ["not-a-url", "file:///tmp/models", "https://user:password@example.test", "https://example.test/#secret", "https://example.test/?token=value"]) {
      expect(providerDraftIssue({ ...input, apiBase })).toEqual({ field: "base", copyKey: "settings.providers.errors.address" });
    }
    expect(providerDraftIssue(input)).toBeNull();
    expect(providerDraftIssue({ ...input, defaultModel: "" })?.field).toBe("default-model");
    expect(providerDraftIssue({ ...input, defaultModel: "" }, false)).toBeNull();
    for (const apiKeyEnv of ["INVALID NAME", "lowercase_key", "A".repeat(257)]) {
      expect(providerDraftIssue({ ...input, apiKeyEnv })?.field).toBe("key-env");
    }
    expect(providerDraftIssue({ ...input, providerType: "fake", apiBase: "", defaultModel: "fake" })).toBeNull();
  });
});
