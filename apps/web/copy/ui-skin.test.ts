import { describe, expect, it } from "vitest";

import { LOCALES, getDictionary, lookupCopy } from "../copy";

describe("ui skin copy", () => {
  it("exposes both visual directions in every locale", () => {
    for (const locale of LOCALES) {
      const dict = getDictionary(locale);
      expect(lookupCopy(dict, "uiSkin.warm")).not.toBe("uiSkin.warm");
      expect(lookupCopy(dict, "uiSkin.graphite")).not.toBe("uiSkin.graphite");
      expect(lookupCopy(dict, "uiSkin.label")).not.toBe("uiSkin.label");
    }
  });
});

describe("default ui skin", () => {
  it("defaults to graphite until the user stores another preference", async () => {
    const { FALLBACK_UI_SKIN } = await import("../shell/ui-skin");
    expect(FALLBACK_UI_SKIN.skin).toBe("graphite");
  });
});
