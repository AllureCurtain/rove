"use client";

import {
  CheckIcon,
  Cross2Icon,
  MagnifyingGlassIcon,
  Pencil2Icon,
  PlusIcon,
  ReloadIcon,
  TrashIcon,
} from "@radix-ui/react-icons";
import { type FormEvent, useEffect, useState } from "react";

import { ProductApiError } from "../product/product-client";
import type {
  CreateProductMcpServerRequest,
  ProductMcpProbeResponse,
  ProductMcpServerConfig,
  ProductMcpTransport,
  UpdateProductMcpServerRequest,
} from "./settings-platform-api-types";
import type { SettingsPlatformClient } from "./settings-platform-client";

const TRANSPORT_LABELS: Record<ProductMcpTransport, string> = {
  stdio: "stdio",
  streamable_http: "Streamable HTTP",
  sse: "Legacy SSE",
};

export interface McpServerDraft {
  name: string;
  enabled: boolean;
  transport: ProductMcpTransport;
  command: string;
  argsText: string;
  envNamesText: string;
  url: string;
  timeoutMs: string;
}

interface ProbeState {
  busy: boolean;
  response?: ProductMcpProbeResponse;
  error?: string;
}

export function createEmptyMcpServerDraft(): McpServerDraft {
  return {
    name: "",
    enabled: true,
    transport: "stdio",
    command: "",
    argsText: "",
    envNamesText: "",
    url: "",
    timeoutMs: "30000",
  };
}

export function mcpServerDraftFromConfig(
  server: ProductMcpServerConfig,
): McpServerDraft {
  return {
    name: server.name,
    enabled: server.enabled,
    transport: server.transport,
    command: server.command ?? "",
    argsText: server.args.join("\n"),
    envNamesText: server.env_names.join("\n"),
    url: server.url ?? "",
    timeoutMs: String(server.request_timeout_ms),
  };
}

function nonEmptyLines(value: string): string[] {
  return value
    .split(/\r?\n/u)
    .map((line) => line.trim())
    .filter(Boolean);
}

export function mcpServerRequestFromDraft(
  draft: McpServerDraft,
): CreateProductMcpServerRequest {
  const common = {
    name: draft.name.trim(),
    enabled: draft.enabled,
    transport: draft.transport,
    request_timeout_ms: Number(draft.timeoutMs),
  };
  if (draft.transport !== "stdio") {
    return {
      ...common,
      args: [],
      env_names: [],
      url: draft.url.trim(),
    };
  }
  return {
    ...common,
    command: draft.command.trim(),
    args: nonEmptyLines(draft.argsText),
    env_names: nonEmptyLines(draft.envNamesText),
  };
}

function updateRequest(
  request: CreateProductMcpServerRequest,
): UpdateProductMcpServerRequest {
  const { name: _name, ...update } = request;
  return update;
}

function describeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function describeMcpProbeFailure(error: unknown): string {
  const code = error instanceof ProductApiError ? error.code : null;
  switch (code) {
    case "product_mcp_environment_missing":
      return "A configured environment variable is unavailable to the API process.";
    case "product_mcp_spawn_failed":
      return "The stdio server could not be started. Check its command and arguments.";
    case "product_mcp_timeout":
      return "The MCP connection test timed out.";
    case "product_mcp_transport":
      return "The MCP transport closed or returned an unsuccessful response.";
    case "product_mcp_protocol_mismatch":
      return "The endpoint did not return a compatible MCP tool catalog.";
    case "product_mcp_no_tools":
      return "The MCP server connected but returned no tools.";
    default:
      return describeError(error);
  }
}

function sortServers(servers: ProductMcpServerConfig[]): ProductMcpServerConfig[] {
  return [...servers].sort((left, right) => left.name.localeCompare(right.name));
}

export function MCPSettings({
  client,
  workspaceId,
}: {
  client: SettingsPlatformClient;
  workspaceId: string | null;
}) {
  const [servers, setServers] = useState<ProductMcpServerConfig[]>([]);
  const [loading, setLoading] = useState(workspaceId !== null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [draft, setDraft] = useState<McpServerDraft>(createEmptyMcpServerDraft);
  const [editingName, setEditingName] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState<string | null>(null);
  const [deletingName, setDeletingName] = useState<string | null>(null);
  const [rowErrors, setRowErrors] = useState<Record<string, string>>({});
  const [probes, setProbes] = useState<Record<string, ProbeState>>({});

  async function loadServers(): Promise<void> {
    if (!workspaceId) {
      setServers([]);
      setLoading(false);
      return;
    }
    setLoading(true);
    setLoadError(null);
    try {
      const response = await client.listMcpServers(workspaceId);
      setServers(sortServers(response.servers));
    } catch (error) {
      setLoadError(describeError(error));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    let active = true;
    if (!workspaceId) {
      setServers([]);
      setLoading(false);
      return () => {
        active = false;
      };
    }
    setLoading(true);
    setLoadError(null);
    void client
      .listMcpServers(workspaceId)
      .then((response) => {
        if (active) {
          setServers(sortServers(response.servers));
        }
      })
      .catch((error: unknown) => {
        if (active) {
          setLoadError(describeError(error));
        }
      })
      .finally(() => {
        if (active) {
          setLoading(false);
        }
      });
    return () => {
      active = false;
    };
  }, [client, workspaceId]);

  function resetDraft(): void {
    setDraft(createEmptyMcpServerDraft());
    setEditingName(null);
    setFormError(null);
  }

  function startEdit(server: ProductMcpServerConfig): void {
    setDraft(mcpServerDraftFromConfig(server));
    setEditingName(server.name);
    setFormError(null);
  }

  function chooseTransport(transport: ProductMcpTransport): void {
    setDraft((current) =>
      transport === "stdio"
        ? { ...current, transport, url: "" }
        : {
            ...current,
            transport,
            command: "",
            argsText: "",
            envNamesText: "",
          },
    );
  }

  async function handleSave(event: FormEvent<HTMLFormElement>): Promise<void> {
    event.preventDefault();
    if (!workspaceId) {
      return;
    }
    setSaving(true);
    setFormError(null);
    try {
      const request = mcpServerRequestFromDraft(draft);
      const saved = editingName
        ? await client.updateMcpServer(
            workspaceId,
            editingName,
            updateRequest(request),
          )
        : await client.createMcpServer(workspaceId, request);
      setServers((current) =>
        sortServers([
          ...current.filter((server) => server.name !== saved.name),
          saved,
        ]),
      );
      setProbes((current) => {
        const next = { ...current };
        delete next[saved.name];
        return next;
      });
      resetDraft();
    } catch (error) {
      setFormError(describeError(error));
    } finally {
      setSaving(false);
    }
  }

  async function handleDelete(name: string): Promise<void> {
    if (!workspaceId || deletingName) {
      return;
    }
    setDeletingName(name);
    setRowErrors((current) => {
      const next = { ...current };
      delete next[name];
      return next;
    });
    try {
      await client.deleteMcpServer(workspaceId, name);
      setServers((current) => current.filter((server) => server.name !== name));
      setConfirmingDelete(null);
      if (editingName === name) {
        resetDraft();
      }
    } catch (error) {
      setRowErrors((current) => ({ ...current, [name]: describeError(error) }));
    } finally {
      setDeletingName(null);
    }
  }

  async function handleProbe(name: string): Promise<void> {
    if (!workspaceId || probes[name]?.busy) {
      return;
    }
    setProbes((current) => ({ ...current, [name]: { busy: true } }));
    try {
      const response = await client.probeMcpServer(workspaceId, name);
      setProbes((current) => ({
        ...current,
        [name]: { busy: false, response },
      }));
    } catch (error) {
      setProbes((current) => ({
        ...current,
        [name]: { busy: false, error: describeMcpProbeFailure(error) },
      }));
    }
  }

  if (!workspaceId) {
    return (
      <div className="settings-card" aria-label="MCP servers">
        <h2>MCP servers</h2>
        <p className="settings-inline-note">
          Select a workspace to manage its MCP servers.
        </p>
      </div>
    );
  }

  return (
    <div className="mcp-settings" aria-label="MCP servers">
      <form className="settings-card" onSubmit={(event) => void handleSave(event)} aria-busy={saving}>
        <div className="settings-card__heading">
          <h2>{editingName ? `Edit ${editingName}` : "Add MCP server"}</h2>
          {editingName ? (
            <button type="button" className="secondary" onClick={resetDraft} disabled={saving}>
              <Cross2Icon /> Cancel edit
            </button>
          ) : null}
        </div>

        <div className="field-grid">
          <div className="field">
            <label htmlFor="mcp-server-name">Server name</label>
            <input
              id="mcp-server-name"
              value={draft.name}
              disabled={saving || editingName !== null}
              maxLength={64}
              autoComplete="off"
              onChange={(event) =>
                setDraft((current) => ({ ...current, name: event.target.value }))
              }
            />
          </div>
          <div className="field settings-number-field">
            <label htmlFor="mcp-timeout">Connection timeout (ms)</label>
            <input
              id="mcp-timeout"
              type="number"
              min={100}
              max={120000}
              step={100}
              value={draft.timeoutMs}
              disabled={saving}
              onChange={(event) =>
                setDraft((current) => ({
                  ...current,
                  timeoutMs: event.target.value,
                }))
              }
            />
          </div>
        </div>

        <div className="mcp-form-options">
          <div className="settings-segmented" role="group" aria-label="MCP transport">
            {(["stdio", "streamable_http", "sse"] as const).map((transport) => (
              <button
                key={transport}
                type="button"
                data-active={draft.transport === transport}
                aria-pressed={draft.transport === transport}
                disabled={saving}
                onClick={() => chooseTransport(transport)}
              >
                {TRANSPORT_LABELS[transport]}
              </button>
            ))}
          </div>
          <label className="settings-checkbox" htmlFor="mcp-enabled">
            <input
              id="mcp-enabled"
              type="checkbox"
              checked={draft.enabled}
              disabled={saving}
              onChange={(event) =>
                setDraft((current) => ({
                  ...current,
                  enabled: event.target.checked,
                }))
              }
            />
            <span>Enabled</span>
          </label>
        </div>

        {draft.transport === "stdio" ? (
          <>
            <div className="field">
              <label htmlFor="mcp-command">Command</label>
              <input
                id="mcp-command"
                value={draft.command}
                disabled={saving}
                autoComplete="off"
                onChange={(event) =>
                  setDraft((current) => ({
                    ...current,
                    command: event.target.value,
                  }))
                }
              />
            </div>
            <div className="field-grid">
              <div className="field">
                <label htmlFor="mcp-args">Arguments (one per line)</label>
                <textarea
                  id="mcp-args"
                  rows={3}
                  value={draft.argsText}
                  disabled={saving}
                  onChange={(event) =>
                    setDraft((current) => ({
                      ...current,
                      argsText: event.target.value,
                    }))
                  }
                />
              </div>
              <div className="field">
                <label htmlFor="mcp-env-names">Environment names (one per line)</label>
                <textarea
                  id="mcp-env-names"
                  rows={3}
                  value={draft.envNamesText}
                  disabled={saving}
                  autoComplete="off"
                  onChange={(event) =>
                    setDraft((current) => ({
                      ...current,
                      envNamesText: event.target.value,
                    }))
                  }
                />
              </div>
            </div>
          </>
        ) : (
          <div className="field">
            <label htmlFor="mcp-sse-url">
              {draft.transport === "streamable_http"
                ? "Streamable HTTP endpoint URL"
                : "Legacy SSE URL"}
            </label>
            <input
              id="mcp-sse-url"
              type="url"
              value={draft.url}
              disabled={saving}
              autoComplete="off"
              onChange={(event) =>
                setDraft((current) => ({ ...current, url: event.target.value }))
              }
            />
          </div>
        )}

        <div className="field-actions">
          <button type="submit" disabled={saving}>
            {editingName ? <CheckIcon /> : <PlusIcon />}
            {saving ? "Saving..." : editingName ? "Save changes" : "Add server"}
          </button>
        </div>
        {formError ? <div className="chat-error" role="alert">{formError}</div> : null}
      </form>

      <div className="settings-card" aria-busy={loading}>
        <div className="settings-card__heading">
          <h2>Workspace servers</h2>
          <button
            type="button"
            className="icon-button secondary"
            aria-label="Refresh MCP servers"
            title="Refresh MCP servers"
            disabled={loading}
            onClick={() => void loadServers()}
          >
            <ReloadIcon />
          </button>
        </div>
        {loadError ? (
          <div className="chat-error" role="alert">
            {loadError}
          </div>
        ) : null}
        {loading && servers.length === 0 ? (
          <p className="settings-inline-note">Loading MCP servers...</p>
        ) : null}
        {!loading && servers.length === 0 ? (
          <p className="settings-inline-note">No MCP servers in this workspace.</p>
        ) : null}
        {servers.length > 0 ? (
          <div className="profile-list">
            {servers.map((server) => {
              const probe = probes[server.name];
              const confirming = confirmingDelete === server.name;
              const deleting = deletingName === server.name;
              return (
                <div className="profile-row mcp-server-row" key={server.name}>
                  <div>
                    <div className="mcp-server-heading">
                      <strong>{server.name}</strong>
                      <span className="mcp-server-status" data-enabled={server.enabled}>
                        {server.enabled ? "Enabled" : "Disabled"}
                      </span>
                    </div>
                    <span>
                      {server.transport === "stdio"
                        ? `stdio · ${server.command} · ${server.args.length} args · ${server.env_names.length} env names`
                        : `${TRANSPORT_LABELS[server.transport]} · ${server.url}`}
                      {` · ${server.request_timeout_ms} ms`}
                      {server.transport_deprecated ? " · deprecated transport" : ""}
                    </span>
                    {rowErrors[server.name] ? (
                      <div className="chat-error" role="alert">
                        {rowErrors[server.name]}
                      </div>
                    ) : null}
                    {probe?.error ? (
                      <div className="chat-error" role="alert">
                        {probe.error}
                      </div>
                    ) : null}
                    {probe?.response ? (
                      <div className="mcp-probe-result" aria-live="polite">
                        <span>
                          {probe.response.tools.length} tools · tested {new Date(probe.response.tested_at).toLocaleString()}
                        </span>
                        <ul className="mcp-tool-list">
                          {probe.response.tools.map((tool) => (
                            <li key={tool.name}>
                              <strong>{tool.name}</strong>
                              <span>{tool.description || "No description"}</span>
                            </li>
                          ))}
                        </ul>
                      </div>
                    ) : null}
                    {confirming ? (
                      <div className="settings-inline-confirm" role="alert">
                        <span>Remove {server.name} from this workspace?</span>
                        <div className="field-actions">
                          <button
                            type="button"
                            className="secondary"
                            disabled={deleting}
                            onClick={() => setConfirmingDelete(null)}
                          >
                            <Cross2Icon /> Cancel
                          </button>
                          <button
                            type="button"
                            className="danger"
                            disabled={deleting}
                            onClick={() => void handleDelete(server.name)}
                          >
                            <TrashIcon /> {deleting ? "Removing..." : "Confirm remove"}
                          </button>
                        </div>
                      </div>
                    ) : null}
                  </div>
                  <div className="field-actions">
                    <button
                      type="button"
                      className="secondary"
                      disabled={probe?.busy || deleting}
                      onClick={() => void handleProbe(server.name)}
                    >
                      <MagnifyingGlassIcon /> {probe?.busy ? "Testing..." : "Test"}
                    </button>
                    <button
                      type="button"
                      className="secondary"
                      disabled={saving || deleting}
                      onClick={() => startEdit(server)}
                    >
                      <Pencil2Icon /> Edit
                    </button>
                    <button
                      type="button"
                      className="danger"
                      disabled={deleting || confirming}
                      onClick={() => setConfirmingDelete(server.name)}
                    >
                      <TrashIcon /> Remove
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        ) : null}
      </div>
    </div>
  );
}
