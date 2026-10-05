import { describe, expect, it } from "vitest";

import type { ProductModelDescriptor } from "../product/product-api-types";
import { quickModelOptions, quickModelReasoning } from "./QuickModelControl";

const catalog: ProductModelDescriptor[] = [
  {
    id: "provider-model-a",
    supports_reasoning: true,
    supported_reasoning: ["low", "medium", "high"],
  },
  {
    id: "provider-model-b",
    supports_reasoning: false,
    supported_reasoning: [],
    reasoning_unavailable_reason: "Reasoning is disabled by this provider.",
  },
];

describe("QuickModelControl inventory", () => {
  it("uses the real provider catalog while retaining the current model", () => {
    expect(
      quickModelOptions("current-model", ["stale-profile-default"], catalog),
    ).toEqual(["current-model", "provider-model-a", "provider-model-b"]);
  });

  it("derives reasoning availability from the selected provider model", () => {
    expect(quickModelReasoning("provider-model-a", catalog).available).toBe(true);
    expect(quickModelReasoning("provider-model-b", catalog)).toEqual({
      available: false,
      reason: "Reasoning is disabled by this provider.",
    });
    expect(quickModelReasoning("unknown-model", catalog).available).toBe(false);
    expect(quickModelReasoning("provider-model-a", undefined).available).toBe(false);
  });
});
