import { describe, expect, it } from "vitest";

import type { ProductPreferences } from "../product/product-api-types";
import {
  selectionFromPreferences,
  toPreferencesRequest,
} from "./server-product-state";

function preferences(
  overrides: Partial<ProductPreferences> = {},
): ProductPreferences {
  return {
    schema_version: 1,
    revision: 7,
    theme: "dark",
    default_approval_policy: "never",
    ...overrides,
  };
}

describe("server product preference mapping", () => {
  it("uses the durable default approval without an explicit provider selection", () => {
    expect(selectionFromPreferences(preferences())).toMatchObject({
      mode: "default",
      approval: "never",
    });
  });

  it("keeps an explicit provider selection authoritative", () => {
    expect(
      selectionFromPreferences(
        preferences({
          provider_selection: {
            profile_id: "profile-1",
            model: "gpt-test",
            approval: "auto",
            max_steps: 12,
          },
        }),
      ),
    ).toEqual({
      mode: "profile",
      profileId: "profile-1",
      model: "gpt-test",
      approval: "auto",
      maxSteps: 12,
    });
  });

  it("writes the confirmed revision and default approval into CAS requests", () => {
    expect(toPreferencesRequest(preferences())).toEqual({
      schema_version: 1,
      expected_revision: 7,
      theme: "dark",
      default_approval_policy: "never",
      active_workspace_id: undefined,
      active_session_id: undefined,
      provider_selection: undefined,
    });
  });
});
