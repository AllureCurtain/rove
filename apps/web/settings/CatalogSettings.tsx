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
  useMemo,
  useRef,
  useState,
} from "react";

import { createProductApiClient } from "../product/product-client";
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
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) {
    return value;
  }
  return new Date(timestamp)
    .toISOString()
    .replace("T", " ")
    .replace(/\.\d{3}Z$/, " UTC");
}

function workspaceKindLabel(kind: WorkspaceRecord["kind"]): string {
  switch (kind) {
    case "repo":
      return "Repo";
    case "task":
      return "Task";
    default:
      return "Folder";
  }
}

function StatusChip({ status }: { status: SessionRecord["status"] }) {
  if (status === "idle") {
    return null;
  }
  const label = status === "needs_attention" ? "Needs attention" :
    status === "running" ? "Running" : "Error";
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
}: WorkspaceSettingsProps) {
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
      <h1>Workspace / Paths</h1>
      <p className="lede">
        Manage the durable workspace catalog and the local roots available to rove.
      </p>

      <div className="settings-card">
        <h2>Path rules</h2>
        <div className="placeholder-note">
          Add workspaces from the workspace sidebar using an absolute local path. Folder roots
          expose that directory; Repo roots identify a repository checkout. Removing an entry
          removes its sessions from the product catalog, but never deletes files from disk.
        </div>
      </div>

      <div className="settings-card" aria-busy={sortedWorkspaces.some((item) => actions.isBusy(item.id))}>
        <h2>Known workspaces</h2>
        {sortedWorkspaces.length === 0 ? (
          <p style={{ margin: 0, color: "var(--muted)" }}>
            No workspaces are registered yet. Add an absolute Folder or Repo path from the main
            workspace sidebar.
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
                  className="profile-row"
                  key={workspace.id}
                  aria-current={active ? "true" : undefined}
                  style={{ alignItems: "flex-start" }}
                >
                  <div style={{ minWidth: 0, overflowWrap: "anywhere" }}>
                    <strong>{workspace.displayName}{active ? " (active)" : ""}</strong>
                    <span style={{ display: "block", marginTop: 3 }}>
                      {workspaceKindLabel(workspace.kind)}
                      {workspace.pinned ? " · Pinned" : ""}
                      {` · Opened ${formatTimestamp(workspace.lastOpenedAt)}`}
                    </span>
                    <span style={{ display: "block", marginTop: 3 }} title={workspace.rootPath}>
                      {workspace.rootPath}
                    </span>
                    {error ? (
                      <div className="chat-error" role="alert" style={{ marginTop: 8 }}>
                        {error}
                      </div>
                    ) : null}
                    {confirming ? (
                      <div className="placeholder-note" role="alert" style={{ marginTop: 8 }}>
                        Remove this workspace and its sessions from the catalog? Local files are
                        not deleted.
                        <div className="field-actions" style={{ marginTop: 10 }}>
                          <button
                            type="button"
                            className="secondary"
                            disabled={busy}
                            onClick={() => setConfirmingRemovalId(null)}
                          >
                            <Cross2Icon /> Cancel
                          </button>
                          <button
                            type="button"
                            className="danger"
                            disabled={busy}
                            onClick={() => void handleRemove(workspace.id)}
                          >
                            <TrashIcon /> {busy ? "Removing…" : "Confirm remove"}
                          </button>
                        </div>
                      </div>
                    ) : null}
                  </div>
                  <div className="field-actions" style={{ justifyContent: "flex-end" }}>
                    {!active ? (
                      <button
                        type="button"
                        className="secondary"
                        disabled={busy}
                        onClick={() => void actions.run(workspace.id, () => onSelectWorkspace(workspace.id))}
                      >
                        Open
                      </button>
                    ) : null}
                    <button
                      type="button"
                      className="secondary"
                      disabled={busy}
                      onClick={() => void actions.run(workspace.id, () => onTogglePin(workspace.id))}
                    >
                      {workspace.pinned ? <DrawingPinFilledIcon /> : <DrawingPinIcon />}
                      {workspace.pinned ? "Unpin" : "Pin"}
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
                      <TrashIcon /> Remove
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
      className="profile-row"
      aria-current={active ? "true" : undefined}
      aria-busy={busy}
      style={{ alignItems: "flex-start" }}
    >
      <div style={{ minWidth: 0, flex: "1 1 260px" }}>
        <div style={{ display: "flex", alignItems: "center", flexWrap: "wrap", gap: 8 }}>
          <strong style={{ overflowWrap: "anywhere" }}>
            {session.title}{active ? " (active)" : ""}
          </strong>
          <StatusChip status={session.status} />
        </div>
        <span style={{ display: "block", marginTop: 3 }}>
          Updated {formatTimestamp(session.updatedAt)}
          {session.hasDurableTurn ? " · Durable history" : " · No completed turn"}
        </span>
        {editing ? (
          <form onSubmit={(event) => void handleRename(event)} style={{ marginTop: 10 }}>
            <div className="field">
              <label htmlFor={`session-title-${session.id}`}>Session name</label>
              <input
                id={`session-title-${session.id}`}
                value={title}
                maxLength={200}
                disabled={busy}
                autoFocus
                onChange={(event) => setTitle(event.target.value)}
              />
            </div>
            <div className="field-actions" style={{ marginTop: 8 }}>
              <button type="submit" disabled={busy || title.trim().length === 0}>
                <CheckIcon /> {busy ? "Saving…" : "Save"}
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
                <Cross2Icon /> Cancel
              </button>
            </div>
          </form>
        ) : null}
        {confirmingDelete ? (
          <div className="placeholder-note" role="alert" style={{ marginTop: 10 }}>
            Delete this session from the durable catalog? Running or unresolved sessions may be
            rejected by the API.
            <div className="field-actions" style={{ marginTop: 10 }}>
              <button
                type="button"
                className="secondary"
                disabled={busy}
                onClick={() => setConfirmingDelete(false)}
              >
                <Cross2Icon /> Cancel
              </button>
              <button
                type="button"
                className="danger"
                disabled={busy}
                onClick={() => void handleDelete()}
              >
                <TrashIcon /> {busy ? "Deleting…" : "Confirm delete"}
              </button>
            </div>
          </div>
        ) : null}
        {error ? (
          <div className="chat-error" role="alert" style={{ marginTop: 8 }}>
            {error}
          </div>
        ) : null}
      </div>
      <div className="field-actions" style={{ justifyContent: "flex-end" }}>
        {!active ? (
          <button
            type="button"
            className="secondary"
            disabled={busy || !canSelect}
            onClick={() => void actions.run(session.id, () => onSelectSession(session.workspaceId, session.id))}
          >
            Open
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
          <Pencil2Icon /> Rename
        </button>
        <button
          type="button"
          className="secondary"
          disabled={busy}
          onClick={() => void handleExport()}
        >
          <DownloadIcon /> Evidence export
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
          <TrashIcon /> Delete
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
  const groups = useMemo(
    () => groupCatalogSessions(workspaces, sessions),
    [sessions, workspaces],
  );

  return (
    <div className="settings-panel">
      <h1>Sessions</h1>
      <p className="lede">
        Rename, open, export redacted session evidence, or remove durable conversation entries grouped by workspace.
      </p>
      {groups.length === 0 ? (
        <div className="settings-card">
          <h2>No sessions</h2>
          <p style={{ margin: 0, color: "var(--muted)" }}>
            Sessions appear here after a workspace is opened.
          </p>
        </div>
      ) : (
        groups.map((group) => (
          <div className="settings-card" key={group.workspaceId}>
            <div>
              <h2>{group.workspace?.displayName ?? "Unavailable workspace"}</h2>
              <p style={{ margin: "4px 0 0", color: "var(--muted)", fontSize: "0.8rem" }}>
                {group.workspace
                  ? `${workspaceKindLabel(group.workspace.kind)} · ${group.sessions.length} session${group.sessions.length === 1 ? "" : "s"}`
                  : `Workspace ${group.workspaceId} is no longer present in the catalog.`}
              </p>
            </div>
            {group.sessions.length === 0 ? (
              <p style={{ margin: 0, color: "var(--muted)" }}>No sessions in this workspace.</p>
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
