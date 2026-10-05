"use client";

import {
  type MouseEvent,
  type ReactNode,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import {
  DotsHorizontalIcon,
  DrawingPinFilledIcon,
} from "@radix-ui/react-icons";

import { useCopy } from "../copy/CopyProvider";
import { useArmedDelete } from "../shell/use-armed-delete";
import type { SessionRecord, WorkspaceRecord } from "../state/product-types";
import { sessionHref } from "../state/product-route";
import { SessionHoverCard } from "./SessionHoverCard";
import {
  SESSION_HOVER_CARD_SCOPE_SELECTOR,
} from "./session-hover-card";
import {
  sessionResultMark,
  sessionStatusLabel,
  shortId,
} from "./session-labels";
import { useSessionHoverCard } from "./use-session-hover-card";

/** A plain click navigates; a modified click feeds the multi-select model. */
export interface SessionRowClickModifiers {
  additive: boolean;
  range: boolean;
}

export interface SessionRowActions {
  onTogglePin: (sessionId: string) => void;
  onRename?: (sessionId: string, title: string) => Promise<boolean>;
  onArchive: (sessionId: string, archived: boolean) => void;
  onDelete: (sessionId: string) => void;
  onBranch: (sessionId: string) => void;
  /** Evidence export (design §8/A11); absent while no host handles it. */
  onExport?: (sessionId: string) => void;
  /** Batch variants used while the row is part of an active multi-select. */
  onBatchArchive?: (archived: boolean) => void;
  onBatchDelete?: () => void;
  onClearSelection?: () => void;
}

interface SessionRowProps extends SessionRowActions {
  session: SessionRecord;
  /** Owning project; a catalog edge case can leave it momentarily absent. */
  workspace?: WorkspaceRecord;
  /** §7.2: the secondary line is the project name outside project groups. */
  showProject: boolean;
  parentAvailable?: boolean;
  parentTitle?: string | null;
  active: boolean;
  /** Optimistically painted while a selection's navigation is in flight. */
  painted: boolean;
  pinned: boolean;
  multiSelected: boolean;
  selectedCount: number;
  /** True when every selected session is archived — the batch arms Restore. */
  selectionAllArchived: boolean;
  queueCount: number;
  disabled: boolean;
  onSelect: (
    session: SessionRecord,
    modifiers: SessionRowClickModifiers | null,
    /** Keyboard activation skips the paint barrier (`event.detail === 0`). */
    immediate: boolean,
  ) => void;
  /** Project-zone branch children render as a nested list under the row. */
  children?: ReactNode;
}

const MENU_WIDTH_PX = 216;
const MENU_MARGIN_PX = 8;

/**
 * The §7.2 session row: `status · [pin] · title · project · ⋯`, used by the
 * Pinned strip, the time-grouped Sessions zone, and project bodies. The row
 * carries every session affordance — inline rename, the ⋯/context menu with
 * armed delete, the delayed hover card — so the zones only differ in grouping.
 */
export function SessionRow({
  session,
  workspace,
  showProject,
  parentAvailable = true,
  parentTitle,
  active,
  painted,
  pinned,
  multiSelected,
  selectedCount,
  selectionAllArchived,
  queueCount,
  disabled,
  onSelect,
  onTogglePin,
  onRename,
  onArchive,
  onDelete,
  onBranch,
  onExport,
  onBatchArchive,
  onBatchDelete,
  onClearSelection,
  children,
}: SessionRowProps) {
  const { t } = useCopy();
  const hoverCard = useSessionHoverCard(session.id);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [renaming, setRenaming] = useState(false);
  const [renameDraft, setRenameDraft] = useState("");
  const [renameSaving, setRenameSaving] = useState(false);
  const [renameError, setRenameError] = useState<string | null>(null);
  const renameInputRef = useRef<HTMLInputElement>(null);
  const selected = painted;
  const resultMark = sessionResultMark(session, active, t);

  useEffect(() => {
    if (renaming) {
      renameInputRef.current?.focus();
      renameInputRef.current?.select();
    }
  }, [renaming]);

  function startRename() {
    if (!onRename) {
      return;
    }
    setRenameDraft(session.title);
    setRenameError(null);
    setRenaming(true);
  }

  function cancelRename() {
    setRenaming(false);
    setRenameError(null);
  }

  async function commitRename() {
    if (!onRename) {
      return;
    }
    const title = renameDraft.trim();
    if (!title) {
      setRenameError(t("workspace.sessionRenameEmpty"));
      return;
    }
    if (title === session.title) {
      cancelRename();
      return;
    }
    setRenameSaving(true);
    try {
      const persisted = await onRename(session.id, title);
      if (persisted) {
        setRenaming(false);
        setRenameError(null);
      } else {
        setRenameError(t("workspace.sessionRenameFailed"));
      }
    } catch {
      setRenameError(t("workspace.sessionRenameFailed"));
    } finally {
      setRenameSaving(false);
    }
  }

  function handleClick(event: MouseEvent<HTMLButtonElement>) {
    const modifiers: SessionRowClickModifiers = {
      additive: event.ctrlKey || event.metaKey,
      range: event.shiftKey,
    };
    onSelect(
      session,
      modifiers.additive || modifiers.range ? modifiers : null,
      event.detail === 0,
    );
  }

  const batch = multiSelected && selectedCount > 1;
  const workspaceName = workspace?.displayName ?? "";
  const subtitle = showProject
    ? workspaceName
    : session.parentSessionId
      ? forkPointLabel(session, parentAvailable, t)
      : null;

  return (
    <li
      className="session-branch"
      data-forked={session.parentSessionId ? "true" : undefined}
      data-orphaned={
        session.parentSessionId && !parentAvailable ? "true" : undefined
      }
    >
      {renaming ? (
        <div className="session-item session-item--editing">
          <input
            ref={renameInputRef}
            type="text"
            value={renameDraft}
            onChange={(event) => setRenameDraft(event.target.value)}
            disabled={renameSaving}
            aria-label={t("workspace.sessionRename")}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                void commitRename();
              } else if (event.key === "Escape") {
                event.preventDefault();
                cancelRename();
              }
            }}
            onBlur={() => {
              // Blur commits, matching the plan's contract; Escape still
              // cancels because the keydown clears the editor first.
              if (!renameSaving) void commitRename();
            }}
          />
          {renameError ? (
            <p className="session-item__error" role="alert">{renameError}</p>
          ) : null}
        </div>
      ) : (
        <button
          type="button"
          className="session-item"
          data-active={selected || undefined}
          data-multi-selected={multiSelected || undefined}
          data-archived={session.archived || undefined}
          aria-current={active ? "page" : undefined}
          title={sessionAriaLabel(session, workspaceName, parentAvailable, t)}
          data-status={session.status}
          ref={hoverCard.anchorRef}
          aria-describedby={hoverCard.open ? hoverCard.cardId : undefined}
          onMouseEnter={hoverCard.rowProps.onMouseEnter}
          onMouseLeave={hoverCard.rowProps.onMouseLeave}
          onClick={handleClick}
          onDoubleClick={startRename}
          onContextMenu={(event) => {
            event.preventDefault();
            setMenu({ x: event.clientX, y: event.clientY });
          }}
          aria-label={sessionAriaLabel(session, workspaceName, parentAvailable, t)}
          disabled={disabled}
        >
          {/* Status slot first (§7.2): shape+motion, never colour alone. */}
          <span
            className="session-item__status"
            data-status={session.status}
            aria-hidden="true"
          >
            {session.status === "running" ? (
              <span className="session-item__spinner" />
            ) : session.status === "needs_attention" ? (
              <span className="session-item__warning" />
            ) : session.status === "error" ? (
              <span className="session-item__error-dot" />
            ) : null}
          </span>
          {pinned ? (
            <DrawingPinFilledIcon className="session-item__pinmark" aria-hidden="true" />
          ) : null}
          <span className="session-item__title">
            <span>{session.title}</span>
            {subtitle ? (
              <small className="session-item__lineage">{subtitle}</small>
            ) : null}
          </span>
          {/* §3.1 queue-count slot: empty until the catalog exposes per-session
              queue depth; the active session's queue arrives via prop. */}
          {queueCount > 0 ? (
            <span
              className="session-item__queue"
              title={t("workspace.sessionQueueCount", { count: queueCount })}
            >
              {queueCount}
            </span>
          ) : null}
          {session.status !== "idle" ? (
            <span className="session-badge" data-status={session.status}>
              {sessionStatusLabel(session.status, t)}
            </span>
          ) : null}
          {resultMark ? (
            <span
              className="session-item__dot"
              data-tone={resultMark.tone}
              role="status"
              title={resultMark.label}
            />
          ) : null}
        </button>
      )}
      <span className="session-item__actions">
        <button
          type="button"
          className="icon-button session-item__menu-button"
          aria-label={t("workspace.sessionActions", { title: session.title })}
          aria-haspopup="menu"
          aria-expanded={menu !== null}
          disabled={disabled}
          onClick={(event) => {
            const rect = event.currentTarget.getBoundingClientRect();
            setMenu((current) =>
              current === null
                ? { x: rect.right - MENU_WIDTH_PX, y: rect.bottom + 4 }
                : null,
            );
          }}
        >
          <DotsHorizontalIcon />
        </button>
      </span>
      {menu !== null ? (
        <SessionRowMenu
          origin={menu}
          batch={batch}
          selectedCount={selectedCount}
          selectionAllArchived={selectionAllArchived}
          session={session}
          pinned={pinned}
          canRename={onRename !== undefined}
          canBranch={session.status === "idle" && Boolean(session.activeRunId)}
          disabled={disabled}
          onClose={() => setMenu(null)}
          onRename={() => {
            setMenu(null);
            startRename();
          }}
          onTogglePin={() => {
            setMenu(null);
            onTogglePin(session.id);
          }}
          onArchive={(archived) => {
            setMenu(null);
            onArchive(session.id, archived);
          }}
          onBranch={() => {
            setMenu(null);
            onBranch(session.id);
          }}
          onExport={
            onExport
              ? () => {
                  setMenu(null);
                  onExport(session.id);
                }
              : undefined
          }
          onBatchArchive={(archived) => {
            setMenu(null);
            onBatchArchive?.(archived);
          }}
          onBatchDelete={() => onBatchDelete?.()}
          onDelete={() => {
            setMenu(null);
            onDelete(session.id);
          }}
          onClearSelection={() => {
            setMenu(null);
            onClearSelection?.();
          }}
        />
      ) : null}
      {children}
      {hoverCard.open ? (
        <SessionHoverCard
          id={hoverCard.cardId}
          session={session}
          workspaceRootPath={workspace?.rootPath ?? ""}
          parentTitle={parentTitle}
          anchor={hoverCard.anchor}
          host={hoverCard.portalHost}
          cardProps={hoverCard.cardProps}
          onOpen={() => {
            hoverCard.dismiss();
            onSelect(session, null, true);
          }}
        />
      ) : null}
    </li>
  );
}

const ARM_DELETE = "delete";
const ARM_BATCH_DELETE = "batch-delete";

/**
 * The row's ⋯/context menu (§7.3). Fixed-positioned and portaled into the app
 * frame, so the rail's scroll clipping cannot crop it and a right-click can
 * place it at the pointer. In multi-select it degrades to the batch actions.
 */
function SessionRowMenu({
  origin,
  batch,
  selectedCount,
  selectionAllArchived,
  session,
  pinned,
  canRename,
  canBranch,
  disabled,
  onClose,
  onRename,
  onTogglePin,
  onArchive,
  onBranch,
  onExport,
  onBatchArchive,
  onBatchDelete,
  onDelete,
  onClearSelection,
}: {
  origin: { x: number; y: number };
  batch: boolean;
  selectedCount: number;
  selectionAllArchived: boolean;
  session: SessionRecord;
  pinned: boolean;
  canRename: boolean;
  canBranch: boolean;
  disabled: boolean;
  onClose: () => void;
  onRename: () => void;
  onTogglePin: () => void;
  onArchive: (archived: boolean) => void;
  onBranch: () => void;
  onExport?: () => void;
  onBatchArchive: (archived: boolean) => void;
  onBatchDelete: () => void;
  onDelete: () => void;
  onClearSelection: () => void;
}) {
  const { t } = useCopy();
  const { armed, setArmed } = useArmedDelete();
  const [copied, setCopied] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState(origin);

  // Clamp into the viewport once the real menu height is measurable.
  useLayoutEffect(() => {
    const node = menuRef.current;
    if (!node) {
      return;
    }
    const rect = node.getBoundingClientRect();
    const x = Math.max(
      MENU_MARGIN_PX,
      Math.min(origin.x, window.innerWidth - rect.width - MENU_MARGIN_PX),
    );
    const y = Math.max(
      MENU_MARGIN_PX,
      Math.min(origin.y, window.innerHeight - rect.height - MENU_MARGIN_PX),
    );
    if (x !== origin.x || y !== origin.y) {
      setPosition({ x, y });
    }
  }, [origin]);

  useEffect(() => {
    menuRef.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    function outside(event: PointerEvent) {
      if (
        event.target instanceof Node &&
        !menuRef.current?.contains(event.target)
      ) {
        onClose();
      }
    }
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [onClose]);

  const host =
    typeof document === "undefined"
      ? null
      : (document.querySelector<HTMLElement>(SESSION_HOVER_CARD_SCOPE_SELECTOR) ??
        document.body);
  if (host === null) {
    return null;
  }

  return createPortal(
    <div
      ref={menuRef}
      className="workspace-actions__menu session-row-menu"
      role="menu"
      aria-label={
        batch
          ? t("workspace.sessionBatchActions", { count: selectedCount })
          : t("workspace.sessionActions", { title: session.title })
      }
      style={{ left: position.x, top: position.y }}
      onBlur={(event) => {
        // Blur disarms the armed delete and closes the menu — a half-armed
        // destructive item must not survive the menu losing focus.
        if (!event.currentTarget.contains(event.relatedTarget as Node)) {
          setArmed(null);
          onClose();
        }
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          onClose();
          return;
        }
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
      {batch ? (
        <>
          <MenuItem
            disabled={disabled}
            onClick={() => onBatchArchive(!selectionAllArchived)}
          >
            {selectionAllArchived
              ? t("workspace.sessionBatchRestore", { count: selectedCount })
              : t("workspace.sessionBatchArchive", { count: selectedCount })}
          </MenuItem>
          <MenuItem
            disabled={disabled}
            armed={armed === ARM_BATCH_DELETE}
            onClick={() => {
              if (armed !== ARM_BATCH_DELETE) {
                setArmed(ARM_BATCH_DELETE);
                return;
              }
              onBatchDelete();
            }}
          >
            {armed === ARM_BATCH_DELETE
              ? t("workspace.sessionBatchDeleteArmed", { count: selectedCount })
              : t("workspace.sessionBatchDelete", { count: selectedCount })}
          </MenuItem>
          {onClearSelection ? (
            <MenuItem onClick={onClearSelection}>
              {t("workspace.sessionClearSelection")}
            </MenuItem>
          ) : null}
        </>
      ) : (
        <>
          {canRename ? (
            <MenuItem onClick={onRename}>
              {t("workspace.sessionRename")}
            </MenuItem>
          ) : null}
          <MenuItem onClick={onTogglePin}>
            {pinned ? t("workspace.sessionUnpin") : t("workspace.sessionPin")}
          </MenuItem>
          <MenuItem
            disabled={disabled}
            onClick={() => onArchive(!session.archived)}
          >
            {session.archived
              ? t("workspace.sessionRestore")
              : t("workspace.sessionArchive")}
          </MenuItem>
          <MenuItem
            disabled={
              disabled ||
              !canBranch
            }
            title={
              canBranch ? undefined : t("workspace.sessionBranchUnavailable")
            }
            onClick={onBranch}
          >
            {t("workspace.sessionBranch")}
          </MenuItem>
          <MenuItem
            onClick={() => {
              const link = new URL(
                sessionHref(session.workspaceId, session.id),
                window.location.origin,
              ).toString();
              void navigator.clipboard
                ?.writeText(link)
                .then(() => setCopied(true))
                .catch(() => undefined);
            }}
          >
            {copied ? t("common.copied") : t("workspace.sessionCopyLink")}
          </MenuItem>
          {onExport ? (
            <MenuItem onClick={onExport}>
              {t("workspace.sessionExport")}
            </MenuItem>
          ) : null}
          <div className="session-row-menu__divider" role="separator" />
          <MenuItem
            disabled={disabled}
            armed={armed === ARM_DELETE}
            destructive
            onClick={() => {
              // First click arms and relabels; the second executes. The arm
              // expires by itself and a menu blur disarms it.
              if (armed !== ARM_DELETE) {
                setArmed(ARM_DELETE);
                return;
              }
              onDelete();
            }}
          >
            {armed === ARM_DELETE
              ? t("workspace.sessionDeleteArmed")
              : t("workspace.sessionDelete")}
          </MenuItem>
        </>
      )}
    </div>,
    host,
  );
}

function MenuItem({
  disabled,
  armed,
  destructive,
  title,
  onClick,
  children,
}: {
  disabled?: boolean;
  armed?: boolean;
  destructive?: boolean;
  title?: string;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      className="ghost"
      disabled={disabled}
      data-armed={armed ? "true" : undefined}
      data-destructive={destructive ? "true" : undefined}
      title={title}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

function sessionAriaLabel(
  session: SessionRecord,
  workspaceName: string,
  parentAvailable: boolean,
  t: (path: string) => string,
): string {
  const parts: string[] = [];
  if (session.parentSessionId) {
    parts.push(
      parentAvailable
        ? t("workspace.forked")
        : t("workspace.forkedRemovedParent"),
    );
  }
  parts.push(session.title);
  if (workspaceName) {
    parts.push(workspaceName);
  }
  if (session.archived) {
    parts.push(t("workspace.group.archived"));
  }
  if (session.status !== "idle") {
    parts.push(sessionStatusLabel(session.status, t));
  }
  return parts.join(", ");
}

function forkPointLabel(
  session: SessionRecord,
  parentAvailable: boolean,
  t: (path: string) => string,
): string {
  const source = session.forkPointRunId
    ? shortId(session.forkPointRunId)
    : t("inspector.notAvailable");
  const sequence = session.forkPointSeq
    ? `event ${session.forkPointSeq}`
    : t("inspector.notAvailable");
  return `${parentAvailable ? t("workspace.forkPrefix") : t("workspace.parentRemoved")} · ${source} · ${sequence}`;
}
