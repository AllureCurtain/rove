import { describe, expect, it } from "vitest";

import { describeProviderProbeFailure } from "./provider-settings-model";

describe("provider settings failure reasons", () => {
  it.each([
    ["provider_timeout", "timed out"],
    ["provider_authentication", "authentication failed"],
    ["provider_rate_limited", "rate limit reached"],
    ["provider_protocol_mismatch", "compatible model catalog"],
    ["provider_no_models", "no usable models"],
    ["provider_transport", "could not be reached"],
  ])("distinguishes %s", (code, expected) => {
    expect(describeProviderProbeFailure({ code, message: "generic" }).toLowerCase())
      .toContain(expected);
  });

  it("preserves an ordinary client error", () => {
    expect(describeProviderProbeFailure(new Error("profile is invalid")))
      .toBe("profile is invalid");
  });
});
