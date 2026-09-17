import { DEFAULT_LOCALE, type Locale } from "./locales";
import zhCN, { type CopyDictionary } from "./zh-CN";
import enUS from "./en-US";

export { DEFAULT_LOCALE, LOCALES, LOCALE_STORAGE_KEY, LOCALE_BOOTSTRAP_SCRIPT, isLocale, readStoredLocale, writeStoredLocale } from "./locales";
export type { Locale } from "./locales";
export type { CopyDictionary } from "./zh-CN";

const dictionaries: Record<Locale, CopyDictionary> = {
  "zh-CN": zhCN,
  "en-US": enUS,
};

export function getDictionary(locale: Locale): CopyDictionary {
  return dictionaries[locale] ?? dictionaries[DEFAULT_LOCALE];
}

export function interpolate(
  template: string,
  params?: Record<string, string | number>,
): string {
  if (!params) {
    return template;
  }
  return template.replace(/\{\{(\w+)\}\}/g, (_match, key: string) => {
    const value = params[key];
    return value === undefined ? "" : String(value);
  });
}

export function lookupCopy(dict: CopyDictionary, path: string): string {
  const parts = path.split(".");
  let current: unknown = dict;
  for (const part of parts) {
    if (current && typeof current === "object" && part in current) {
      current = (current as Record<string, unknown>)[part];
    } else {
      return path;
    }
  }
  return typeof current === "string" ? current : path;
}

export function createTranslator(locale: Locale) {
  const dict = getDictionary(locale);
  return function t(
    path: string,
    params?: Record<string, string | number>,
  ): string {
    return interpolate(lookupCopy(dict, path), params);
  };
}
