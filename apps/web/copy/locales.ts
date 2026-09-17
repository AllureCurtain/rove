export const LOCALE_STORAGE_KEY = "rove.locale";

export type Locale = "zh-CN" | "en-US";

export function resolveDefaultLocale(value: unknown): Locale {
  return value === "en-US" ? "en-US" : "zh-CN";
}

export const DEFAULT_LOCALE: Locale = resolveDefaultLocale(
  process.env.ROVE_DEFAULT_LOCALE,
);

export const LOCALES: readonly Locale[] = ["zh-CN", "en-US"] as const;

export function isLocale(value: unknown): value is Locale {
  return value === "zh-CN" || value === "en-US";
}

export function readStoredLocale(): Locale | null {
  if (typeof window === "undefined") {
    return null;
  }
  try {
    const raw = window.localStorage.getItem(LOCALE_STORAGE_KEY);
    return isLocale(raw) ? raw : null;
  } catch {
    return null;
  }
}

export function writeStoredLocale(locale: Locale): void {
  if (typeof window === "undefined") {
    return;
  }
  try {
    window.localStorage.setItem(LOCALE_STORAGE_KEY, locale);
  } catch {
    // Locale caching is optional; in-memory state remains authoritative.
  }
}

export const LOCALE_BOOTSTRAP_SCRIPT = `try{var l=localStorage.getItem(${JSON.stringify(
  LOCALE_STORAGE_KEY,
)});if(l==="zh-CN"||l==="en-US"){document.documentElement.lang=l}}catch{}`;
