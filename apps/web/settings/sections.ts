export const SETTINGS_SECTIONS = [
  { id: "general", label: "General" },
  { id: "providers", label: "Providers & Models" },
  { id: "tools", label: "Tools & Approvals" },
  { id: "workspace", label: "Workspace / Paths" },
  { id: "memory", label: "Memory" },
  { id: "sessions", label: "Sessions" },
  { id: "keyboard", label: "Keyboard shortcuts" },
  { id: "advanced", label: "Advanced / Developer" },
  { id: "about", label: "About / Runtime" },
] as const;

export type SettingsSectionId = (typeof SETTINGS_SECTIONS)[number]["id"];
