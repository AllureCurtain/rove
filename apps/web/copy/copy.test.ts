import { describe, expect, it } from "vitest";

import enUS from "./en-US";
import {
  createTranslator,
  getDictionary,
  interpolate,
  lookupCopy,
} from "./index";
import { DEFAULT_LOCALE, isLocale, LOCALES, resolveDefaultLocale } from "./locales";
import zhCN from "./zh-CN";

function collectPaths(
  value: unknown,
  prefix = "",
): string[] {
  if (typeof value === "string") {
    return [prefix];
  }
  if (!value || typeof value !== "object") {
    return [];
  }
  return Object.entries(value as Record<string, unknown>).flatMap(([key, child]) =>
    collectPaths(child, prefix ? `${prefix}.${key}` : key),
  );
}

describe("copy dictionaries", () => {
  it("exposes the same key tree in every locale", () => {
    const zhPaths = collectPaths(zhCN).sort();
    const enPaths = collectPaths(enUS).sort();
    expect(enPaths).toEqual(zhPaths);
  });

  it("has no empty strings", () => {
    for (const locale of LOCALES) {
      const dict = getDictionary(locale);
      for (const path of collectPaths(dict)) {
        expect(lookupCopy(dict, path), `${locale} ${path}`).not.toBe("");
      }
    }
  });

  it("uses the validated build-time default", () => {
    expect(DEFAULT_LOCALE).toBe(resolveDefaultLocale(process.env.ROVE_DEFAULT_LOCALE));
    expect(getDictionary(DEFAULT_LOCALE)).toBe(DEFAULT_LOCALE === "en-US" ? enUS : zhCN);
  });

  it("supports an English build and falls back safely to Chinese", () => {
    expect(resolveDefaultLocale("en-US")).toBe("en-US");
    for (const value of [undefined, null, "", "fr-FR", "zh-CN", "<script>"]) {
      expect(resolveDefaultLocale(value)).toBe("zh-CN");
    }
  });

  it("keeps internal developer jargon out of product copy", () => {
    const banned =
      /product_session|canonical |continuity:|cache key|prompt hash|key_present|wire /i;
    for (const locale of LOCALES) {
      const dict = getDictionary(locale);
      for (const path of collectPaths(dict)) {
        const value = lookupCopy(dict, path);
        expect(banned.test(value), `${locale} ${path}: ${value}`).toBe(false);
      }
    }
  });

  it("interpolates named parameters", () => {
    expect(interpolate("Connected — {{count}} models", { count: 12 })).toBe(
      "Connected — 12 models",
    );
  });

  it("translates dotted paths", () => {
    const t = createTranslator("en-US");
    expect(t("settings.providers.localDemo")).toBe("Local demo");
    expect(t("missing.path")).toBe("missing.path");
  });

  it("recognizes supported locales only", () => {
    expect(isLocale("zh-CN")).toBe(true);
    expect(isLocale("en-US")).toBe(true);
    expect(isLocale("fr-FR")).toBe(false);
  });
});
