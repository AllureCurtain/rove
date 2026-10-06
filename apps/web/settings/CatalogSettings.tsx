"use client";

import {
  CheckIcon,
  Cross2Icon,
  DownloadIcon,
  DrawingPinFilledIcon,
  DrawingPinIcon,
  Pencil2Icon,
  TrashIcon,
} from "@radix-ui/react-icons";
import {
  type FormEvent,
  type ReactNode,
  useMemo,
  useRef,
  useState,
} from "react";

import { createProductApiClient } from "../product/product-client";
import { useCopy } from "../copy/CopyProvider";
import { formatUtcTimestamp } from "../lib/format-timestamp";
import { downloadEvidenceFile } from "../product/evidence-export";
import type { SessionRecord, WorkspaceRecord } from "../state/product-types";
import {
  groupCatalogSessions,
  resolveSessionSelection,
  sortCatalogWorkspaces,
} from "./catalog-settings-model";

type MaybePromise = void | Promise<unknown>;

export interface WorkspaceSettingsProps {
  workspaces: readonly WorkspaceRecord[];
  activeWorkspaceId: string | null;
  onSelectWorkspace: (workspaceId: string) => MaybePromise;
  onTogglePin: (workspaceId: string) => MaybePromise;
  onRemoveWorkspace: (workspaceId: string) => MaybePromise;
  projectTrust?: ReactNode;
}

export interface SessionsSettingsProps {
  workspaces: readonly WorkspaceRecord[];
  sessions: readonly SessionRecord[];
  activeSessionId: string | null;
  onSelectSession: (workspaceId: string, sessionId: string) => MaybePromise;
  onRenameSession: (sessionId: string, title: string) => MaybePromise;
  onDeleteSession: (sessionId: string) => MaybePromise;
}

interface GuardedActionState {
  isBusy: (itemId: string) => boolean;
  errorFor: (itemId: string) => string | null;
  clearError: (itemId: string) => void;
  run: (itemId: string, action: () => MaybePromise) => Promise<boolean>;
}

function describeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function useGuardedItemActions(): GuardedActionState {
  const inFlightRef = useRef(new Set<string>());
  const [busyItems, setBusyItems] = useState<ReadonlySet<string>>(() => new Set());
  const [errors, setErrors] = useState<Record<string, string>>({});

  function isBusy(itemId: string): boolean {
    return busyItems.has(itemId);
  }

  function errorFor(itemId: string): string | null {
    return errors[itemId] ?? null;
  }

  function clearError(itemId: string): void {
    setErrors((current) => {
      if (!(itemId in current)) {
        return current;
      }
      const next = { ...current };
      delete next[itemId];
      return next;
    });
  }

  async function run(itemId: string, action: () => MaybePromise): Promise<boolean> {
    if (inFlightRef.current.has(itemId)) {
      return false;
    }
    inFlightRef.current.add(itemId);
    setBusyItems((current) => new Set(current).add(itemId));
    clearError(itemId);
    try {
      await action();
      return true;
    } catch (error) {
      setErrors((current) => ({ ...current, [itemId]: describeError(error) }));
      return false;
    } finally {
      inFlightRef.current.delete(itemId);
      setBusyItems((current) => {
        const next = new Set(current);
        next.delete(itemId);
        return next;
      });
    }
  }

  return { isBusy, errorFor, clearError, run };
}

function formatTimestamp(value: string): string {
  return formatUtcTimestamp(value);
}

function workspaceKindLabel(
  kind: WorkspaceRecord["kind"],
  t: (path: string) => string,
): string {
  switch (kind) {
    case "repo":
      return t("workspace.kindRepo");
    case "task":
      return t("workspace.kindTask");
    default:
      return t("workspace.kindFolder");
  }
}

function StatusChip({ status }: { status: SessionRecord["status"] }) {
  const { t } = useCopy();
  if (status === "idle") {
    return null;
  }
  const label = status === "needs_attention" ? t("workspace.needsAttention") :
    status === "running" ? t("workspace.running") : t("workspace.statusError");
  const className = status === "running" ? "status-chip status-chip--running" :
    "status-chip status-chip--failed";
  return <span className={className}>{label}</span>;
}

export function WorkspaceSettings({
  workspaces,
  activeWorkspaceId,
  onSelectWorkspace,
  onTogglePin,
  onRemoveWorkspace,
  projectTrust,
}: WorkspaceSettingsProps) {
  const { t } = useCopy();
  const sortedWorkspaces = useMemo(() => sortCatalogWorkspaces(workspaces), [workspaces]);
  const actions = useGuardedItemActions();
  const [confirmingRemovalId, setConfirmingRemovalId] = useState<string | null>(null);

  async function handleRemove(workspaceId: string): Promise<void> {
    const succeeded = await actions.run(workspaceId, () => onRemoveWorkspace(workspaceId));
    if (succeeded) {
      setConfirmingRemovalId(null);
    }
  }

  return (
    <div className="settings-panel">
      <h1>{t("settings.sectionWorkspace")}</h1>
      <p className="lede">
        {t("workspace.pathRulesBody")}
      </p>

      <div className="settings-card">
        <h2>{t("workspace.pathRules")}</h2>
        <div className="placeholder-note">
          {t("workspace.pathRulesBody")}
        </div>
      </div>

      {projectTrust}

      <div className="settings-card" aria-busy={sortedWorkspaces.some((item) => actions.isBusy(item.id))}>
        <h2>{t("workspace.known")}</h2>
        {sortedWorkspaces.length === 0 ? (
          <p className="settings-text">
            {t("workspace.none")}
          </p>
        ) : (
          <div className="profile-list">
            {sortedWorkspaces.map((workspace) => {
              const busy = actions.isBusy(workspace.id);
              const error = actions.errorFor(workspace.id);
              const active = workspace.id === activeWorkspaceId;
              const confirming = confirmingRemovalId === workspace.id;
              return (
                <div
                  className="profile-row profile-row--top"
                  key={workspace.id}
                  aria-current={active ? "true" : undefined}
                >
                  <div>
                    <strong>{workspace.displayName}{active ? t("workspace.active") : ""}</strong>
                    <span className="profile-row__meta">
                      {workspaceKindLabel(workspace.kind, t)}
                      {workspace.pinned ? ` · ${t("workspace.pinned")}` : ""}
                      {` · ${t("workspace.opened", { time: formatTimestamp(workspace.lastOpenedAt) })}`}
                    </span>
                    <span className="profile-row__meta" title={workspace.rootPath}>
                      {workspace.rootPath}
                    </span>
                    {error ? (
                      <div className="chat-error settings-stack-8" role="alert">
                        {error}
                      </div>
                    ) : null}
                    {confirming ? (
                      <div className="placeholder-note settings-stack-8" role="alert">
                        {t("workspace.removeConfirm")}
                        <div className="field-actions settings-stack-10">
                          <button
                            type="button"
                            className="secondary"
                            disabled={busy}
                            onClick={() => setConfirmingRemovalId(null)}
                          >
                            <Cross2Icon /> {t("common.cancel")}
                          </button>
                          <button
                            type="button"
                            className="danger"
                            disabled={busy}
                            onClick={() => void handleRemove(workspace.id)}
                          >
                            <TrashIcon /> {busy ? t("common.loading") : t("workspace.remove")}
                          </button>
                        </div>
                      </div>
                    ) : null}
                  </div>
                  <div className="field-actions field-actions--end">
                    {!active ? (
                      <button
                        type="button"
                        className="secondary"
                        disabled={busy}
                        onClick={() => void actions.run(workspace.id, () => onSelectWorkspace(workspace.id))}
                      >
                        {t("empty.open")}
                      </button>
                    ) : null}
                    <button
                      type="button"
                      className="secondary"
                      disabled={busy}
                      onClick={() => void actions.run(workspace.id, () => onTogglePin(workspace.id))}
                    >
                      {workspace.pinned ? <DrawingPinFilledIcon /> : <DrawingPinIcon />}
                      {workspace.pinned ? t("workspace.unpin") : t("workspace.pin")}
                    </button>
                    <button
                      type="button"
                      className="danger"
                      disabled={busy || confirming}
                      onClick={() => {
                        actions.clearError(workspace.id);
                        setConfirmingRemovalId(workspace.id);
                      }}
                    >
                      <TrashIcon /> {t("workspace.remove")}
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

function SessionRow({
  session,
  active,
  canSelect,
  onSelectSession,
  onRenameSession,
  onDeleteSession,
}: {
  session: SessionRecord;
  active: boolean;
  canSelect: boolean;
  onSelectSession: SessionsSettingsProps["onSelectSession"];
  onRenameSession: SessionsSettingsProps["onRenameSession"];
  onDeleteSession: SessionsSettingsProps["onDeleteSession"];
}) {
  const { t } = useCopy();
  const actions = useGuardedItemActions();
  const busy = actions.isBusy(session.id);
  const error = actions.errorFor(session.id);
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState(session.title);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const client = useMemo(() => createProductApiClient(), []);

  async function handleRename(event: FormEvent<HTMLFormElement>): Promise<void> {
    event.preventDefault();
    const nextTitle = title.trim();
    if (!nextTitle) {
      return;
    }
    if (nextTitle === session.title) {
      setEditing(false);
      return;
    }
    const succeeded = await actions.run(session.id, () => onRenameSession(session.id, nextTitle));
    if (succeeded) {
      setEditing(false);
    }
  }

  async function handleDelete(): Promise<void> {
    const succeeded = await actions.run(session.id, () => onDeleteSession(session.id));
    if (succeeded) {
      setConfirmingDelete(false);
    }
  }

  async function handleExport(): Promise<void> {
    await actions.run(session.id, async () => {
      const download = await client.exportSessionEvidence(session.id, "json");
      downloadEvidenceFile(download);
    });
  }

  return (
    <div
      className="profile-row profile-row--top"
      aria-current={active ? "true" : undefined}
      aria-busy={busy}
    >
      <div className="profile-row__body">
        <div className="profile-row__title">
          <strong className="settings-break">
            {session.title}{active ? t("workspace.active") : ""}
          </strong>
          <StatusChip status={session.status} />
        </div>
        <span className="profile-row__meta">
          {t("workspace.updated", { time: formatTimestamp(session.updatedAt) })}
          {session.hasDurableTurn
            ? ` · ${t("workspace.durableHistory")}`
            : ` · ${t("workspace.noCompletedTurn")}`}
        </span>
        {editing ? (
          <form className="settings-stack-10" onSubmit={(event) => void handleRename(event)}>
            <div className="field">
              <label htmlFor={`session-title-${session.id}`}>{t("workspace.sessionName")}</label>
              <input
                id={`session-title-${session.id}`}
                value={title}
                maxLength={200}
                disabled={busy}
                autoFocus
                onChange={(event) => setTitle(event.target.value)}
              />
            </div>
            <div className="field-actions settings-stack-8">
              <button type="submit" disabled={busy || title.trim().length === 0}>
                <CheckIcon /> {busy ? t("common.saving") : t("common.save")}
              </button>
              <button
                type="button"
                className="secondary"
                disabled={busy}
                onClick={() => {
                  setTitle(session.title);
                  setEditing(false);
                }}
              >
                <Cross2Icon /> {t("common.cancel")}
              </button>
            </div>
          </form>
        ) : null}
        {confirmingDelete ? (
          <div className="placeholder-note settings-stack-10" role="alert">
            {t("workspace.sessionDeleteConfirm")}
            <div className="field-actions settings-stack-10">
              <button
                type="button"
                className="secondary"
                disabled={busy}
                onClick={() => setConfirmingDelete(false)}
              >
                <Cross2Icon /> {t("common.cancel")}
              </button>
              <button
                type="button"
                className="danger"
                disabled={busy}
                onClick={() => void handleDelete()}
              >
                <TrashIcon /> {busy ? t("common.loading") : t("settings.providers.delete")}
              </button>
            </div>
          </div>
        ) : null}
        {error ? (
          <div className="chat-error settings-stack-8" role="alert">
            {error}
          </div>
        ) : null}
      </div>
      <div className="field-actions field-actions--end">
        {!active ? (
          <button
            type="button"
            className="secondary"
            disabled={busy || !canSelect}
            onClick={() => void actions.run(session.id, () => onSelectSession(session.workspaceId, session.id))}
          >
            {t("common.back")}
          </button>
        ) : null}
        <button
          type="button"
          className="secondary"
          disabled={busy || editing || confirmingDelete}
          onClick={() => {
            actions.clearError(session.id);
            setTitle(session.title);
            setEditing(true);
          }}
        >
          <Pencil2Icon /> {t("settings.providers.edit")}
        </button>
        <button
          type="button"
          className="secondary"
          disabled={busy}
          onClick={() => void handleExport()}
        >
          <DownloadIcon /> {t("settings.export.title")}
        </button>
        <button
          type="button"
          className="danger"
          disabled={busy || editing || confirmingDelete}
          onClick={() => {
            actions.clearError(session.id);
            setConfirmingDelete(true);
          }}
        >
          <TrashIcon /> {t("settings.providers.delete")}
        </button>
      </div>
    </div>
  );
}

export function SessionsSettings({
  workspaces,
  sessions,
  activeSessionId,
  onSelectSession,
  onRenameSession,
  onDeleteSession,
}: SessionsSettingsProps) {
  const { t } = useCopy();
  const groups = useMemo(
    () => groupCatalogSessions(workspaces, sessions),
    [sessions, workspaces],
  );

  return (
    <div className="settings-panel">
      <h1>{t("settings.sectionSessions")}</h1>
      <p className="lede">
        {t("settings.lede")}
      </p>
      {groups.length === 0 ? (
        <div className="settings-card">
          <h2>{t("workspace.noSessions")}</h2>
          <p className="settings-text">
            {t("workspace.noSessionsBody")}
          </p>
        </div>
      ) : (
        groups.map((group) => (
          <div className="settings-card" key={group.workspaceId}>
            <div>
              <h2>{group.workspace?.displayName ?? t("workspace.unavailableWorkspace")}</h2>
              <p className="settings-text settings-text--detail">
                {group.workspace
                  ? `${workspaceKindLabel(group.workspace.kind, t)} · ${t(
                      group.sessions.length === 1
                        ? "workspace.sessionCountOne"
                        : "workspace.sessionCountMany",
                      { count: group.sessions.length },
                    )}`
                  : t("workspace.workspaceGone", { id: group.workspaceId })}
              </p>
            </div>
            {group.sessions.length === 0 ? (
              <p className="settings-text">{t("workspace.noSessionsBody")}</p>
            ) : (
              <div className="profile-list">
                {group.sessions.map((session) => {
                  const selection = resolveSessionSelection(workspaces, sessions, session.id);
                  return (
                    <SessionRow
                      key={session.id}
                      session={session}
                      active={session.id === activeSessionId}
                      canSelect={selection !== null}
                      onSelectSession={onSelectSession}
                      onRenameSession={onRenameSession}
                      onDeleteSession={onDeleteSession}
                    />
                  );
                })}
              </div>
            )}
          </div>
        ))
      )}
    </div>
  );
}
