"use client";

import {
  GearIcon,
  MoonIcon,
  SunIcon,
} from "@radix-ui/react-icons";

export function TopBar({
  connectionLabel,
  connectionTone,
  theme,
  onToggleTheme,
  onOpenSettings,
  showSettingsBack,
  onBackToChat,
}: {
  connectionLabel: string;
  connectionTone: "ok" | "working" | "error" | "idle";
  theme: "light" | "dark";
  onToggleTheme: () => void;
  onOpenSettings: () => void;
  showSettingsBack?: boolean;
  onBackToChat?: () => void;
}) {
  return (
    <header className="product-topbar">
      <div className="product-topbar__brand">
        <strong>rove</strong>
        <span>agent</span>
      </div>
      <div className="product-topbar__meta">
        <span className="status-dot" data-tone={connectionTone === "idle" ? undefined : connectionTone} />
        <span>{connectionLabel}</span>
        <button
          type="button"
          className="ghost icon-button"
          onClick={onToggleTheme}
          aria-label={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
        >
          {theme === "dark" ? <SunIcon /> : <MoonIcon />}
        </button>
        {showSettingsBack ? (
          <button type="button" className="secondary" onClick={onBackToChat}>
            Back to chat
          </button>
        ) : (
          <button
            type="button"
            className="ghost icon-button"
            onClick={onOpenSettings}
            aria-label="Open settings"
          >
            <GearIcon />
          </button>
        )}
      </div>
    </header>
  );
}
