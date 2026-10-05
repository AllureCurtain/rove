import { describe, expect, it } from "vitest";

import {
  providerDefaultApiBase,
  providerDefaultKeyEnv,
} from "./provider-store";

describe("provider defaults", () => {
  it("uses the server-compatible empty configuration for fake", () => {
    expect(providerDefaultApiBase("fake")).toBe("");
    expect(providerDefaultKeyEnv("fake")).toBe("");
  });
});
