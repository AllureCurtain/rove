import type { ShortcutBinding } from "./keybindings";

export type KeyboardShortcutActionId =
  | "focus-composer"
  | "new-session"
  | "open-settings"
  | "toggle-inspector"
  | "open-command-palette";

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
  {
    action: "open-command-palette",
    title: "Command palette",
    description: "Search commands, workspaces, sessions and settings.",
    key: "k",
    modifiers: { primary: true, shift: false, alt: false },
    display: "Ctrl / Cmd + K",
    ariaKeyShortcuts: "Control+K Meta+K",
    // Deliberately available while typing: a palette that cannot be opened from
    // the composer is a palette nobody uses.
    allowInEditable: true,
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
  overrides?: Readonly<Record<string, ShortcutBinding | null>>,
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
    KEYBOARD_SHORTCUTS.find((descriptor) => {
      const binding = effectiveShortcutBinding(descriptor, overrides);
      if (binding === null) {
        // Explicitly unbound: the action keeps its entry but no chord.
        return false;
      }
      return (
        binding.key.toLowerCase() === normalizedKey &&
        (binding.primary ? onePrimaryPressed : !anyPrimaryPressed) &&
        binding.shift === event.shiftKey &&
        binding.alt === event.altKey
      );
    }) ?? null;

  if (
    shortcut !== null &&
    !shortcut.allowInEditable &&
    isEditableShortcutTarget(event.target ?? null)
  ) {
    return null;
  }

  return shortcut;
}

/**
 * The binding an action answers to right now: the stored override, null when
 * the reader unbound it, or the built-in default.
 */
export function effectiveShortcutBinding(
  descriptor: KeyboardShortcutDescriptor,
  overrides?: Readonly<Record<string, ShortcutBinding | null>>,
): ShortcutBinding | null {
  if (!overrides || !(descriptor.action in overrides)) {
    return {
      key: descriptor.key,
      primary: descriptor.modifiers.primary,
      shift: descriptor.modifiers.shift,
      alt: descriptor.modifiers.alt,
    };
  }
  return overrides[descriptor.action] ?? null;
}
