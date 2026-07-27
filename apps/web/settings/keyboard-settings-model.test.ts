import { describe, expect, it } from "vitest";

import {
  KEYBOARD_SHORTCUTS,
  isEditableShortcutTarget,
  matchKeyboardShortcut,
  type KeyboardShortcutEventLike,
} from "./keyboard-settings-model";

function keyEvent(
  key: string,
  overrides: Partial<KeyboardShortcutEventLike> = {},
): KeyboardShortcutEventLike {
  return {
    key,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    altKey: false,
    ...overrides,
  };
}

function target(value: object): EventTarget {
  return value as EventTarget;
}

describe("keyboard settings model", () => {
  it("defines unique, documented descriptors for implemented product actions", () => {
    expect(KEYBOARD_SHORTCUTS.map((shortcut) => shortcut.action)).toEqual([
      "focus-composer",
      "new-session",
      "open-settings",
      "toggle-inspector",
    ]);
    expect(new Set(KEYBOARD_SHORTCUTS.map((shortcut) => shortcut.action)).size).toBe(
      KEYBOARD_SHORTCUTS.length,
    );
    expect(
      KEYBOARD_SHORTCUTS.every(
        (shortcut) =>
          shortcut.title.length > 0 &&
          shortcut.description.length > 0 &&
          shortcut.display.length > 0 &&
          shortcut.ariaKeyShortcuts.length > 0,
      ),
    ).toBe(true);
  });

  it("matches slash and primary-modifier shortcuts on Windows and macOS", () => {
    expect(matchKeyboardShortcut(keyEvent("/"))?.action).toBe(
      "focus-composer",
    );
    expect(
      matchKeyboardShortcut(
        keyEvent("Enter", { ctrlKey: true, shiftKey: true }),
      )?.action,
    ).toBe("new-session");
    expect(
      matchKeyboardShortcut(keyEvent(",", { metaKey: true }))?.action,
    ).toBe("open-settings");
    expect(
      matchKeyboardShortcut(keyEvent(".", { ctrlKey: true }))?.action,
    ).toBe("toggle-inspector");
  });

  it("requires an exact modifier match", () => {
    expect(
      matchKeyboardShortcut(
        keyEvent("Enter", { ctrlKey: true, shiftKey: false }),
      ),
    ).toBeNull();
    expect(
      matchKeyboardShortcut(
        keyEvent(",", { ctrlKey: true, metaKey: true }),
      ),
    ).toBeNull();
    expect(
      matchKeyboardShortcut(keyEvent(".", { ctrlKey: true, altKey: true })),
    ).toBeNull();
    expect(matchKeyboardShortcut(keyEvent("/", { shiftKey: true }))).toBeNull();
    expect(
      matchKeyboardShortcut(keyEvent("/", { ctrlKey: true, metaKey: true })),
    ).toBeNull();
  });

  it("ignores repeated, composing, and already handled events", () => {
    expect(matchKeyboardShortcut(keyEvent("/", { repeat: true }))).toBeNull();
    expect(matchKeyboardShortcut(keyEvent("/", { isComposing: true }))).toBeNull();
    expect(
      matchKeyboardShortcut(keyEvent("/", { defaultPrevented: true })),
    ).toBeNull();
  });

  it("recognizes native, contenteditable, and nested editable targets", () => {
    expect(isEditableShortcutTarget(target({ tagName: "TEXTAREA" }))).toBe(true);
    expect(
      isEditableShortcutTarget(target({ tagName: "DIV", isContentEditable: true })),
    ).toBe(true);
    expect(
      isEditableShortcutTarget(
        target({ tagName: "SPAN", closest: () => ({ tagName: "INPUT" }) }),
      ),
    ).toBe(true);
    expect(
      isEditableShortcutTarget(
        target({ tagName: "BUTTON", closest: () => null }),
      ),
    ).toBe(false);
  });

  it("does not dispatch application shortcuts while editing", () => {
    expect(
      matchKeyboardShortcut(
        keyEvent("/", { target: target({ tagName: "INPUT" }) }),
      ),
    ).toBeNull();
    expect(
      matchKeyboardShortcut(
        keyEvent(",", {
          ctrlKey: true,
          target: target({ tagName: "TEXTAREA" }),
        }),
      ),
    ).toBeNull();
  });
});
