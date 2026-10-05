"use client";

import {
  type ReactNode,
  useEffect,
  useRef,
  useState,
} from "react";
import { ChevronDownIcon } from "@radix-ui/react-icons";

import { useCopy } from "../copy/CopyProvider";
import { formatUtcTimestamp } from "../lib/format-timestamp";
import type {
  SessionRecord,
  WorkspaceKind,
  WorkspaceRecord,
} from "../state/product-types";
import { OpenWorkspaceDialog, WorkspaceOpenForm } from "./OpenWorkspaceDialog";

/**
 * The middle column when no session is on screen (design §9.1): the hero — a
 * product mark and one-line prompt — the home variant of the same composer, an
 * inline workspace switcher, and the recent sessions that keep the past one
 * click away. Manual path entry stays available but demoted behind a
 * disclosure, and the sidebar `+` shares the same open-workspace form.
 */
export function HomeSurface({
  workspace,
  workspaces,
  recentSessions,
  workspaceName,
  composer,
  sendBlocked,
  onSelectWorkspace,
  onSelectSession,
  onOpenWorkspace,
  onOpenProviders,
}: {
  /** Where a home send lands; `null` means nothing is open yet. */
  workspace: WorkspaceRecord | null;
  workspaces: readonly WorkspaceRecord[];
  recentSessions: readonly SessionRecord[];
  workspaceName: (workspaceId: string) => string | undefined;
  /** The home-variant Composer, wired by the shell. */
  composer: ReactNode;
  /**
   * §9.2: why the send is blocked and which direct action resolves it. The
   * hint is persistent; `alerted` marks that a submit was attempted, so the
   * row re-announces assertively.
   */
  sendBlocked: {
    reason: string;
    action: "workspace" | "providers";
    alerted: boolean;
  } | null;
  onSelectWorkspace: (workspaceId: string) => void;
  onSelectSession: (workspaceId: string, sessionId: string) => void;
  onOpenWorkspace: (path: string, kind: WorkspaceKind) => void;
  onOpenProviders: () => void;
}) {
  const { t } = useCopy();
  const [dialogOpen, setDialogOpen] = useState(false);

  return (
    <section className="home-surface">
      <header className="home-surface__hero">
        <span
          className="product-topbar__mark home-surface__mark"
          aria-hidden="true"
        >
          R
        </span>
        <h1>{t("home.prompt")}</h1>
        <p className="home-surface__lead">
          {t("home.workspaceLead")}
          <HomeWorkspaceSwitcher
            workspaces={workspaces}
            activeId={workspace?.id ?? null}
            onSelect={onSelectWorkspace}
            onOpenNew={() => setDialogOpen(true)}
          />
        </p>
      </header>
      {composer}
      {sendBlocked ? (
        <div
          className="home-surface__send-hint"
          role={sendBlocked.alerted ? "alert" : "status"}
        >
          <span>{sendBlocked.reason}</span>
          <button
            type="button"
            className="secondary"
            onClick={() => {
              if (sendBlocked.action === "workspace") {
                setDialogOpen(true);
              } else {
                onOpenProviders();
              }
            }}
          >
            {sendBlocked.action === "providers"
              ? t("home.configureModel")
              : t("home.openWorkspace")}
          </button>
        </div>
      ) : null}
      {recentSessions.length > 0 ? (
        <section
          className="home-surface__recents"
          aria-labelledby="home-surface-recents-title"
        >
          <h2 id="home-surface-recents-title">{t("home.recentSessions")}</h2>
          <ul>
            {recentSessions.map((session) => (
              <li key={session.id}>
                <button
                  type="button"
                  className="home-surface__recent"
                  onClick={() =>
                    onSelectSession(session.workspaceId, session.id)
                  }
                >
                  <strong>{session.title}</strong>
                  <span>
                    {workspaceName(session.workspaceId) ??
                      session.workspaceId}
                    {" · "}
                    {formatUtcTimestamp(session.updatedAt)}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      <details className="home-surface__manual">
        <summary>{t("home.manualEntry")}</summary>
        <div className="home-surface__manual-body">
          <WorkspaceOpenForm
            idPrefix="home-workspace"
            submitLabel={t("empty.open")}
            onOpen={onOpenWorkspace}
          />
        </div>
      </details>
      {dialogOpen ? (
        <OpenWorkspaceDialog
          onCancel={() => setDialogOpen(false)}
          onOpen={(path, kind) => {
            setDialogOpen(false);
            onOpenWorkspace(path, kind);
          }}
        />
      ) : null}
    </section>
  );
}

/**
 * The dotted-underline project name in the hero copy (design §9.1). It opens a
 * menu of known workspaces plus the shared open-workspace dialog — the same
 * two choices the rail offers.
 */
function HomeWorkspaceSwitcher({
  workspaces,
  activeId,
  onSelect,
  onOpenNew,
}: {
  workspaces: readonly WorkspaceRecord[];
  activeId: string | null;
  onSelect: (workspaceId: string) => void;
  onOpenNew: () => void;
}) {
  const { t } = useCopy();
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) {
      return;
    }
    menu.current?.querySelector<HTMLButtonElement>("button")?.focus();
    function outside(event: PointerEvent) {
      if (
        event.target instanceof Node &&
        !root.current?.contains(event.target)
      ) {
        setOpen(false);
      }
    }
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open]);

  function close() {
    setOpen(false);
    trigger.current?.focus();
  }

  const active = workspaces.find((workspace) => workspace.id === activeId);
  // Nothing known yet: the control skips the empty menu and opens the shared
  // selection flow directly.
  const actsAsMenu = workspaces.length > 0;

  return (
    <span
      className="home-surface__switcher"
      ref={root}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) {
          setOpen(false);
        }
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape" && open) {
          event.preventDefault();
          event.stopPropagation();
          close();
        }
      }}
    >
      <button
        ref={trigger}
        type="button"
        className="home-surface__switcher-button"
        aria-haspopup={actsAsMenu ? "menu" : undefined}
        aria-expanded={actsAsMenu ? open : undefined}
        onClick={() => {
          if (actsAsMenu) {
            setOpen((value) => !value);
          } else {
            onOpenNew();
          }
        }}
        onKeyDown={(event) => {
          if (actsAsMenu && event.key === "ArrowDown") {
            event.preventDefault();
            setOpen(true);
          }
        }}
      >
        {active?.displayName ?? t("home.chooseWorkspace")}
        <ChevronDownIcon aria-hidden="true" />
      </button>
      {open ? (
        <span
          ref={menu}
          className="home-surface__switcher-menu"
          role="menu"
          aria-label={t("home.switcherLabel")}
          onKeyDown={(event) => {
            const items = Array.from(
              event.currentTarget.querySelectorAll<HTMLButtonElement>(
                "button:not(:disabled)",
              ),
            );
            const index = items.indexOf(
              document.activeElement as HTMLButtonElement,
            );
            const next =
              event.key === "Home"
                ? 0
                : event.key === "End"
                  ? items.length - 1
                  : event.key === "ArrowDown"
                    ? (index + 1) % items.length
                    : event.key === "ArrowUp"
                      ? (index - 1 + items.length) % items.length
                      : null;
            if (next !== null) {
              event.preventDefault();
              items[next]?.focus();
            }
          }}
        >
          {workspaces.map((workspace) => (
            <button
              key={workspace.id}
              type="button"
              role="menuitemradio"
              aria-checked={workspace.id === activeId}
              onClick={() => {
                close();
                onSelect(workspace.id);
              }}
            >
              {workspace.displayName}
            </button>
          ))}
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              close();
              onOpenNew();
            }}
          >
            {t("home.openWorkspace")}
          </button>
        </span>
      ) : null}
    </span>
  );
}
