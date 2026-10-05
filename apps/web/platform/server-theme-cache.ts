export const SERVER_CONFIRMED_THEME_CACHE_KEY =
  "rove.product.server-theme.v1";

export type ServerConfirmedTheme = "light" | "dark";

export function readServerConfirmedTheme(): ServerConfirmedTheme {
  if (typeof document !== "undefined") {
    const applied = document.documentElement.dataset.theme;
    if (applied === "light" || applied === "dark") {
      return applied;
    }
  }
  if (typeof window === "undefined") {
    return "light";
  }
  try {
    const cached = window.localStorage.getItem(SERVER_CONFIRMED_THEME_CACHE_KEY);
    return cached === "dark" ? "dark" : "light";
  } catch {
    return "light";
  }
}

export function cacheServerConfirmedTheme(theme: ServerConfirmedTheme): void {
  if (typeof window === "undefined") {
    return;
  }
  try {
    window.localStorage.setItem(SERVER_CONFIRMED_THEME_CACHE_KEY, theme);
  } catch {
    // Theme caching is optional; API preferences remain authoritative.
  }
}

export const SERVER_THEME_BOOTSTRAP_SCRIPT = `try{var t=localStorage.getItem(${JSON.stringify(
  SERVER_CONFIRMED_THEME_CACHE_KEY,
)});if(t==="light"||t==="dark"){document.documentElement.dataset.theme=t}}catch{}`;
