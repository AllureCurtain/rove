import type { PlatformAdapter, ThemePreference } from "./types";

const THEME_KEY = "rove.theme";

function readStoredTheme(): ThemePreference {
  if (typeof window === "undefined") {
    return "light";
  }
  const raw = window.localStorage.getItem(THEME_KEY);
  if (raw === "light" || raw === "dark" || raw === "system") {
    return raw;
  }
  return "light";
}

function systemTheme(): "light" | "dark" {
  if (typeof window === "undefined" || !window.matchMedia) {
    return "light";
  }
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export const webPlatform: PlatformAdapter = {
  host: "web",

  async pickWorkspacePath(): Promise<string | null> {
    // Web M1: path is entered in the UI; native picker is Desktop-only.
    return null;
  },

  getThemePreference(): ThemePreference {
    return readStoredTheme();
  },

  setThemePreference(theme: ThemePreference): void {
    if (typeof window === "undefined") {
      return;
    }
    window.localStorage.setItem(THEME_KEY, theme);
  },

  resolveTheme(theme: ThemePreference): "light" | "dark" {
    return theme === "system" ? systemTheme() : theme;
  },

  storageGet(key: string): string | null {
    if (typeof window === "undefined") {
      return null;
    }
    return window.localStorage.getItem(key);
  },

  storageSet(key: string, value: string): void {
    if (typeof window === "undefined") {
      return;
    }
    window.localStorage.setItem(key, value);
  },

  storageRemove(key: string): void {
    if (typeof window === "undefined") {
      return;
    }
    window.localStorage.removeItem(key);
  },
};

export function applyDocumentTheme(theme: "light" | "dark"): void {
  if (typeof document === "undefined") {
    return;
  }
  document.documentElement.dataset.theme = theme;
}
