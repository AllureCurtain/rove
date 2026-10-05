"use client";

import {
  GearIcon,
  MoonIcon,
  SunIcon,
} from "@radix-ui/react-icons";
import type { ReactNode } from "react";

import { useCopy } from "../copy/CopyProvider";

/**
 * The page-level bar used by the boot and settings surfaces (design §3.2). The
 * console's own conversation top bar lives in `ConversationTopBar.tsx`, pinned
 * to the middle column.
 */
export function TopBar({
  connectionLabel,
  connectionTone,
  theme,
  onToggleTheme,
  onOpenSettings,
  showSettingsBack,
  onBackToChat,
  notifications,
}: {
  connectionLabel: string;
  connectionTone: "ok" | "working" | "error" | "idle";
  theme: "light" | "dark";
  onToggleTheme: () => void;
  onOpenSettings: () => void;
  showSettingsBack?: boolean;
  onBackToChat?: () => void;
  /** The notification inbox; it owns its own open/closed state. */
  notifications?: ReactNode;
}) {
  const { t } = useCopy();
  return (
    <header className="product-topbar">
      <div className="product-topbar__brand">
        <span className="product-topbar__mark" aria-hidden="true">R</span>
        <strong>{t("chrome.brandName")}</strong>
        <span>{t("chrome.brandTag")}</span>
      </div>
      <div className="product-topbar__meta">
        <span className="status-dot" data-tone={connectionTone === "idle" ? undefined : connectionTone} />
        <span className="product-topbar__connection">{connectionLabel}</span>
        {notifications}
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
