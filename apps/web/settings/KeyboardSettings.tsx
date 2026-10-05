"use client";

import { useCallback, useEffect, useRef, useState } from "react";

import { useCopy } from "../copy/CopyProvider";
import {
  effectiveShortcutBinding,
  KEYBOARD_SHORTCUTS,
  type KeyboardShortcutActionId,
} from "./keyboard-settings-model";
import { formatShortcutBinding, normalizeOverride, type ShortcutBinding } from "./keybindings";
import {
  setKeybindingOverride,
  useKeybindingOverrides,
} from "./use-keybinding-overrides";

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
  "open-command-palette": {
    title: "keyboard.openCommandPalette",
    description: "keyboard.openCommandPaletteDesc",
  },
};

const BARE_MODIFIER_KEYS = new Set([
  "Shift",
  "Control",
  "Alt",
  "Meta",
  "CapsLock",
  "NumLock",
  "ScrollLock",
]);

type RecordError =
  | { kind: "reserved" }
  | { kind: "conflict"; actionId: string };

export function KeyboardSettings() {
  const { t } = useCopy();
  const overrides = useKeybindingOverrides();
  const [recording, setRecording] = useState<KeyboardShortcutActionId | null>(null);
  const [recordError, setRecordError] = useState<RecordError | null>(null);

  const actionTitles = new Map(
    KEYBOARD_SHORTCUTS.map((descriptor) => [
      descriptor.action,
      t(SHORTCUT_COPY[descriptor.action].title),
    ]),
  );

  const handleKeydown = useCallback(
    (event: KeyboardEvent) => {
      const actionId = recording;
      if (!actionId) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      if (event.key === "Escape") {
        setRecording(null);
        return;
      }
      if (BARE_MODIFIER_KEYS.has(event.key)) {
        // A lone modifier is not a chord; keep waiting for the rest.
        return;
      }
      const descriptor = KEYBOARD_SHORTCUTS.find(
        (candidate) => candidate.action === actionId,
      );
      if (!descriptor) {
        setRecording(null);
        return;
      }
      const binding: ShortcutBinding = {
        key: event.key,
        primary: event.ctrlKey || event.metaKey,
        shift: event.shiftKey,
        alt: event.altKey,
      };
      const others = KEYBOARD_SHORTCUTS.filter(
        (candidate) => candidate.action !== actionId,
      ).map((candidate) => ({
        id: candidate.action,
        binding: effectiveShortcutBinding(candidate, overrides)!,
      }));
      const decision = normalizeOverride(binding, { id: descriptor.action, binding: {
        key: descriptor.key,
        primary: descriptor.modifiers.primary,
        shift: descriptor.modifiers.shift,
        alt: descriptor.modifiers.alt,
      } }, others);
      if (decision.rejection) {
        setRecordError(
          decision.rejection.reason === "reserved"
            ? { kind: "reserved" }
            : { kind: "conflict", actionId: decision.rejection.conflictsWith ?? "" },
        );
        setRecording(null);
        return;
      }
      setRecordError(null);
      setRecording(null);
      setKeybindingOverride(actionId, decision.store ?? undefined);
    },
    [overrides, recording],
  );

  useEffect(() => {
    if (!recording) {
      return;
    }
    // Capture phase: the recorder outranks every other handler while active.
    window.addEventListener("keydown", handleKeydown, { capture: true });
    return () => window.removeEventListener("keydown", handleKeydown, { capture: true });
  }, [handleKeydown, recording]);

  const unbind = (actionId: KeyboardShortcutActionId) => {
    setRecordError(null);
    setKeybindingOverride(actionId, null);
  };
  const resetBinding = (actionId: KeyboardShortcutActionId) => {
    setRecordError(null);
    setKeybindingOverride(actionId, undefined);
  };

  return (
    <section className="settings-panel" aria-labelledby="keyboard-settings-title">
      <h1 id="keyboard-settings-title">{t("settings.sectionKeyboard")}</h1>
      <p className="lede">{t("keyboard.lede")}</p>
      {recordError ? (
        <p className="shell-alert" role="alert">
          {recordError.kind === "reserved"
            ? t("keyboard.reserved")
            : t("keyboard.conflict", {
                action: actionTitles.get(
                  recordError.actionId as KeyboardShortcutActionId,
                ) ?? recordError.actionId,
              })}
        </p>
      ) : null}
      <div className="settings-card">
        <h2>{t("keyboard.title")}</h2>
        <div
          className="profile-list"
          role="list"
          aria-label={t("keyboard.title")}
        >
          {KEYBOARD_SHORTCUTS.map((shortcut) => {
            const copy = SHORTCUT_COPY[shortcut.action];
            const binding = effectiveShortcutBinding(shortcut, overrides);
            const overridden = shortcut.action in overrides;
            return (
              <div className="profile-row" role="listitem" key={shortcut.action}>
                <div>
                  <strong>{t(copy.title)}</strong>
                  <span>{t(copy.description)}</span>
                </div>
                {recording === shortcut.action ? (
                  <kbd data-recording="true">{t("keyboard.awaitingKey")}</kbd>
                ) : binding ? (
                  <kbd aria-label={formatShortcutBinding(binding)}>
                    {formatShortcutBinding(binding)}
                    {overridden ? (
                      <small> · {t("keyboard.defaultLabel")}: {shortcut.display}</small>
                    ) : null}
                  </kbd>
                ) : (
                  <kbd data-unbound="true">{t("keyboard.unbound")}</kbd>
                )}
                <div className="keyboard-settings__actions">
                  <button
                    type="button"
                    className="secondary"
                    onClick={() => {
                      setRecordError(null);
                      setRecording(shortcut.action);
                    }}
                    disabled={recording !== null}
                  >
                    {t("keyboard.change")}
                  </button>
                  {binding && overridden ? (
                    <button
                      type="button"
                      className="secondary"
                      onClick={() => resetBinding(shortcut.action)}
                    >
                      {t("keyboard.reset")}
                    </button>
                  ) : null}
                  {binding ? (
                    <button
                      type="button"
                      className="ghost"
                      onClick={() => unbind(shortcut.action)}
                    >
                      {t("keyboard.unbind")}
                    </button>
                  ) : null}
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </section>
  );
}
