"use client";

import {
  GearIcon,
  HamburgerMenuIcon,
  MoonIcon,
  SunIcon,
} from "@radix-ui/react-icons";
import type { Ref } from "react";

import { useCopy } from "../copy/CopyProvider";

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
  const { t } = useCopy();
  return (
    <header className="product-topbar">
      <div className="product-topbar__brand">
        {onToggleWorkspace ? (
          <button
            ref={workspaceButtonRef}
            type="button"
            className="ghost icon-button mobile-only"
            onClick={onToggleWorkspace}
            aria-label={t("nav.expandWorkspace")}
            title={t("nav.workspace")}
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
          aria-label={theme === "dark" ? t("theme.toLight") : t("theme.toDark")}
        >
          {theme === "dark" ? <SunIcon /> : <MoonIcon />}
        </button>
        {showSettingsBack ? (
          <button type="button" className="secondary" onClick={onBackToChat}>
            {t("nav.backToChat")}
          </button>
        ) : (
          <button
            type="button"
            className="ghost icon-button"
            onClick={onOpenSettings}
            aria-label={t("nav.settings")}
          >
            <GearIcon />
          </button>
        )}
      </div>
    </header>
  );
}
