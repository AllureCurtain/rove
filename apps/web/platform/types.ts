/** Host capabilities shared by Web now and Desktop later. */
export type ThemePreference = "light" | "dark" | "system";

export interface PlatformAdapter {
  readonly host: "web" | "desktop";
  /** Open a workspace path. Web uses typed path; Desktop may use a native picker later. */
  pickWorkspacePath(): Promise<string | null>;
  getThemePreference(): ThemePreference;
  setThemePreference(theme: ThemePreference): void;
  resolveTheme(theme: ThemePreference): "light" | "dark";
  storageGet(key: string): string | null;
  storageSet(key: string, value: string): void;
  storageRemove(key: string): void;
}
