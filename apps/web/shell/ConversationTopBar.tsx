"use client";

import {
  HamburgerMenuIcon,
  MagnifyingGlassIcon,
  PinLeftIcon,
  PlusIcon,
} from "@radix-ui/react-icons";
import type { Ref } from "react";

import { useCopy } from "../copy/CopyProvider";

/**
 * The conversation top bar (design §3.2): a 46px in-flow header at the top of
 * the middle column. It holds the session title, the lead rail-expand control
 * that appears while the left rail is collapsed, and the compact action tile
 * (new task + search). Connection health only appears here while it is
 * abnormal — a healthy link is silence, not a badge.
 */
export function ConversationTopBar({
  title,
  subtitle,
  titleRef,
  railLeadVisible,
  onExpandRail,
  onToggleWorkspace,
  workspaceButtonRef,
  connectionLabel,
  connectionAbnormal,
  onNewTask,
  newTaskDisabled,
  onSearch,
  searchDisabled,
}: {
  /** The session title, or the workspace name when no session is open. */
  title: string;
  /** Full workspace path context, exposed as the title tooltip. */
  subtitle?: string;
  /** Focus target for the narrow-screen "select session" flow. */
  titleRef?: Ref<HTMLHeadingElement>;
  /** Desktop only: the collapsed rail grows the 36px lead lane. */
  railLeadVisible?: boolean;
  onExpandRail?: () => void;
  /** Mobile drawer trigger; rendered only while a narrow layout needs it. */
  onToggleWorkspace?: () => void;
  workspaceButtonRef?: Ref<HTMLButtonElement>;
  connectionLabel?: string;
  /** Non-null while the link is degraded — the only time the bar shows it. */
  connectionAbnormal?: boolean;
  onNewTask: () => void;
  newTaskDisabled?: boolean;
  onSearch: () => void;
  searchDisabled?: boolean;
}) {
  const { t } = useCopy();
  return (
    <header
      className="conversation-topbar"
      data-collapsed={railLeadVisible ? "true" : undefined}
    >
      {/*
        The lead lane stays mounted so its slide-in/out tracks the rail's own
        width animation; CSS `visibility` hides it (and drops it from the
        accessibility tree) while the rail is expanded.
      */}
      <div className="ct-lead">
        {onExpandRail ? (
          <button
            type="button"
            className="ct-icon-btn"
            onClick={onExpandRail}
            tabIndex={railLeadVisible ? 0 : -1}
            aria-label={t("nav.expandWorkspace")}
            title={t("nav.expandWorkspace")}
          >
            <PinLeftIcon />
          </button>
        ) : null}
      </div>
      <div className="ct-left">
        {onToggleWorkspace ? (
          <button
            ref={workspaceButtonRef}
            type="button"
            className="ct-icon-btn mobile-only"
            onClick={onToggleWorkspace}
            aria-label={t("nav.expandWorkspace")}
            title={t("nav.workspace")}
          >
            <HamburgerMenuIcon />
          </button>
        ) : null}
        <h1
          ref={titleRef}
          className="ct-title"
          tabIndex={-1}
          title={subtitle}
        >
          {title}
        </h1>
      </div>
      <div className="ct-right">
        {connectionAbnormal && connectionLabel ? (
          <span className="ct-connection">
            <span className="status-dot" data-tone="error" aria-hidden="true" />
            {connectionLabel}
          </span>
        ) : null}
        <div className="ct-actions">
          <button
            type="button"
            className="ct-icon-btn"
            onClick={onNewTask}
            disabled={newTaskDisabled}
            aria-label={t("nav.newTask")}
            title={t("nav.newTask")}
          >
            <PlusIcon />
          </button>
          <button
            type="button"
            className="ct-icon-btn"
            onClick={onSearch}
            disabled={searchDisabled}
            aria-label={t("nav.search")}
            title={t("nav.search")}
          >
            <MagnifyingGlassIcon />
          </button>
        </div>
      </div>
    </header>
  );
}
