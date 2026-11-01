/**
 * A platform-matched example path for the manual-path input. The copy tables
 * live in plain data files, so the platform branch lives here next to them —
 * one Windows-shaped hint on Windows, a POSIX-shaped one everywhere else.
 */
export function defaultWorkspacePathPlaceholder(): string {
  if (typeof navigator !== "undefined" && /win/i.test(navigator.platform ?? "")) {
    return "D:\\path\\to\\project";
  }
  return "/path/to/project";
}
