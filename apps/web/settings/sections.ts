export const SETTINGS_SECTIONS = [
  { id: "general", label: "General" },
  { id: "providers", label: "Providers & Models" },
  { id: "tools", label: "Tools & Approvals" },
  { id: "workspace", label: "Workspace / Paths" },
  { id: "memory", label: "Memory" },
  { id: "sessions", label: "Sessions" },
  { id: "keyboard", label: "Keyboard shortcuts" },
  // Route compatibility only: hidden from navigation; renders General settings.
  { id: "advanced", label: "Advanced" },
  { id: "about", label: "About / Runtime" },
] as const;

export type SettingsSectionId = (typeof SETTINGS_SECTIONS)[number]["id"];

/**
 * Copy key per section: the settings navigation and the command palette must
 * resolve one label for a section, so the mapping lives here instead of being
 * restated per surface.
 */
export const SETTINGS_SECTION_COPY_KEYS: Record<SettingsSectionId, string> = {
  general: "settings.sectionGeneral",
  providers: "settings.sectionProviders",
  tools: "settings.sectionTools",
  workspace: "settings.sectionWorkspace",
  memory: "settings.sectionMemory",
  sessions: "settings.sectionSessions",
  keyboard: "settings.sectionKeyboard",
  advanced: "settings.sectionAdvanced",
  about: "settings.sectionAbout",
};

/** Sections the surfaces offer: `advanced` is a route-compatibility alias. */
export const VISIBLE_SETTINGS_SECTIONS = SETTINGS_SECTIONS.filter(
  (section) => section.id !== "advanced",
);
