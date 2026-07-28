"use client";

import {
  GearIcon,
  HamburgerMenuIcon,
  MoonIcon,
  SunIcon,
} from "@radix-ui/react-icons";
import type { Ref } from "react";

export function TopBar({
  connectionLabel,
  connectionTone,
  theme,
  onToggleTheme,
  onOpenSettings,
  showSettingsBack,
  onBackToChat,
  workspaceButtonRef,
  onToggleWorkspace,
}: {
  connectionLabel: string;
  connectionTone: "ok" | "working" | "error" | "idle";
  theme: "light" | "dark";
  onToggleTheme: () => void;
  onOpenSettings: () => void;
  showSettingsBack?: boolean;
  onBackToChat?: () => void;
  workspaceButtonRef?: Ref<HTMLButtonElement>;
  onToggleWorkspace?: () => void;
}) {
  return (
    <header className="product-topbar">
      <div className="product-topbar__brand">
        {onToggleWorkspace ? (
          <button
            ref={workspaceButtonRef}
            type="button"
            className="ghost icon-button mobile-only"
            onClick={onToggleWorkspace}
            aria-label="Open workspaces"
            title="Open workspaces"
          >
            <HamburgerMenuIcon />
          </button>
        ) : null}
        <span className="product-topbar__mark" aria-hidden="true">R</span>
        <strong>rove</strong>
        <span>local agent</span>
      </div>
      <div className="product-topbar__meta">
        <span className="status-dot" data-tone={connectionTone === "idle" ? undefined : connectionTone} />
        <span className="product-topbar__connection">{connectionLabel}</span>
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
