import { KEYBOARD_SHORTCUTS } from "./keyboard-settings-model";

export function KeyboardSettings() {
  return (
    <section className="settings-panel" aria-labelledby="keyboard-settings-title">
      <h1 id="keyboard-settings-title">Keyboard shortcuts</h1>
      <p className="lede">
        Fast paths for the primary session and navigation actions.
      </p>
      <div className="settings-card">
        <h2>Application shortcuts</h2>
        <div
          className="profile-list"
          role="list"
          aria-label="Application keyboard shortcuts"
        >
          {KEYBOARD_SHORTCUTS.map((shortcut) => (
            <div className="profile-row" role="listitem" key={shortcut.action}>
              <div>
                <strong>{shortcut.title}</strong>
                <span>{shortcut.description}</span>
              </div>
              <kbd aria-label={shortcut.display}>
                {shortcut.display}
              </kbd>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
