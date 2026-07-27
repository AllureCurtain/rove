export type KeyboardShortcutActionId =
  | "focus-composer"
  | "new-session"
  | "open-settings"
  | "toggle-inspector";

export interface KeyboardShortcutDescriptor {
  action: KeyboardShortcutActionId;
  title: string;
  description: string;
  key: string;
  modifiers: {
    primary: boolean;
    shift: boolean;
    alt: boolean;
  };
  display: string;
  ariaKeyShortcuts: string;
  allowInEditable: boolean;
}

export interface KeyboardShortcutEventLike {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  repeat?: boolean;
  defaultPrevented?: boolean;
  isComposing?: boolean;
  target?: EventTarget | null;
}

export const KEYBOARD_SHORTCUTS = [
  {
    action: "focus-composer",
    title: "Focus message composer",
    description: "Move keyboard focus to the message composer.",
    key: "/",
    modifiers: { primary: false, shift: false, alt: false },
    display: "/",
    ariaKeyShortcuts: "/",
    allowInEditable: false,
  },
  {
    action: "new-session",
    title: "New session",
    description: "Create a session in the active workspace.",
    key: "Enter",
    modifiers: { primary: true, shift: true, alt: false },
    display: "Ctrl / Cmd + Shift + Enter",
    ariaKeyShortcuts: "Control+Shift+Enter Meta+Shift+Enter",
    allowInEditable: false,
  },
  {
    action: "open-settings",
    title: "Open settings",
    description: "Open General settings from the product shell.",
    key: ",",
    modifiers: { primary: true, shift: false, alt: false },
    display: "Ctrl / Cmd + ,",
    ariaKeyShortcuts: "Control+, Meta+,",
    allowInEditable: false,
  },
  {
    action: "toggle-inspector",
    title: "Toggle run inspector",
    description: "Show or hide the current run inspector.",
    key: ".",
    modifiers: { primary: true, shift: false, alt: false },
    display: "Ctrl / Cmd + .",
    ariaKeyShortcuts: "Control+. Meta+.",
    allowInEditable: false,
  },
] as const satisfies readonly KeyboardShortcutDescriptor[];

const EDITABLE_SELECTOR = [
  "input",
  "textarea",
  "select",
  '[contenteditable=""]',
  '[contenteditable="true"]',
  '[contenteditable="plaintext-only"]',
  '[role="textbox"]',
  '[role="combobox"]',
  '[role="searchbox"]',
].join(", ");

export function isEditableShortcutTarget(target: EventTarget | null): boolean {
  if (target === null || typeof target !== "object") {
    return false;
  }

  const candidate = target as EventTarget & {
    tagName?: unknown;
    isContentEditable?: unknown;
    closest?: (selector: string) => unknown;
  };
  const tagName =
    typeof candidate.tagName === "string"
      ? candidate.tagName.toLowerCase()
      : "";

  if (
    tagName === "input" ||
    tagName === "textarea" ||
    tagName === "select" ||
    candidate.isContentEditable === true
  ) {
    return true;
  }

  return typeof candidate.closest === "function"
    ? Boolean(candidate.closest(EDITABLE_SELECTOR))
    : false;
}

export function matchKeyboardShortcut(
  event: KeyboardShortcutEventLike,
): KeyboardShortcutDescriptor | null {
  if (
    event.defaultPrevented ||
    event.repeat ||
    event.isComposing
  ) {
    return null;
  }

  const anyPrimaryPressed = event.ctrlKey || event.metaKey;
  const onePrimaryPressed = anyPrimaryPressed && event.ctrlKey !== event.metaKey;
  const normalizedKey = event.key.toLowerCase();

  const shortcut =
    KEYBOARD_SHORTCUTS.find(
      (candidate) =>
        candidate.key.toLowerCase() === normalizedKey &&
        (candidate.modifiers.primary
          ? onePrimaryPressed
          : !anyPrimaryPressed) &&
        candidate.modifiers.shift === event.shiftKey &&
        candidate.modifiers.alt === event.altKey,
    ) ?? null;

  if (
    shortcut !== null &&
    !shortcut.allowInEditable &&
    isEditableShortcutTarget(event.target ?? null)
  ) {
    return null;
  }

  return shortcut;
}
