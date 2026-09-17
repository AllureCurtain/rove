"use client";

import { useCopy } from "../copy/CopyProvider";
import { KEYBOARD_SHORTCUTS, type KeyboardShortcutActionId } from "./keyboard-settings-model";

const SHORTCUT_COPY: Record<
  KeyboardShortcutActionId,
  { title: string; description: string }
> = {
  "focus-composer": {
    title: "keyboard.focusComposer",
    description: "keyboard.focusComposerDesc",
  },
  "new-session": {
    title: "keyboard.newSession",
    description: "keyboard.newSessionDesc",
  },
  "open-settings": {
    title: "keyboard.openSettings",
    description: "keyboard.openSettingsDesc",
  },
  "toggle-inspector": {
    title: "keyboard.toggleInspector",
    description: "keyboard.toggleInspectorDesc",
  },
};

export function KeyboardSettings() {
  const { t } = useCopy();
  return (
    <section className="settings-panel" aria-labelledby="keyboard-settings-title">
      <h1 id="keyboard-settings-title">{t("settings.sectionKeyboard")}</h1>
      <p className="lede">{t("keyboard.lede")}</p>
      <div className="settings-card">
        <h2>{t("keyboard.title")}</h2>
        <div
          className="profile-list"
          role="list"
          aria-label={t("keyboard.title")}
        >
          {KEYBOARD_SHORTCUTS.map((shortcut) => {
            const copy = SHORTCUT_COPY[shortcut.action];
            return (
              <div className="profile-row" role="listitem" key={shortcut.action}>
                <div>
                  <strong>{t(copy.title)}</strong>
                  <span>{t(copy.description)}</span>
                </div>
                <kbd aria-label={shortcut.display}>
                  {shortcut.display}
                </kbd>
              </div>
            );
          })}
        </div>
      </div>
    </section>
  );
}
