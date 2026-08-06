"use client";

import {
  CheckIcon,
  Cross2Icon,
  LockClosedIcon,
  ReloadIcon,
} from "@radix-ui/react-icons";
import { useEffect, useMemo, useState } from "react";

import {
  PRODUCT_TRUST_CAPABILITIES,
  type ProductTrustCapability,
  type ProductTrustDecision,
  type ProductTrustStatus,
} from "./settings-platform-api-types";
import type { SettingsPlatformClient } from "./settings-platform-client";

const CAPABILITY_LABELS: Record<ProductTrustCapability, string> = {
  project_configuration: "Project configuration",
  workspace_instructions: "Workspace instructions",
  mcp_processes: "MCP processes",
  hooks_extensions: "Hooks and extensions",
  provider_credentials: "Provider selectors",
  external_paths: "External paths",
};

export function ProjectTrustSettings({
  client,
  workspaceId,
}: {
  client: SettingsPlatformClient;
  workspaceId: string | null;
}) {
  const [status, setStatus] = useState<ProductTrustStatus | null>(null);
  const [selected, setSelected] = useState<ReadonlySet<ProductTrustCapability>>(
    () => new Set(PRODUCT_TRUST_CAPABILITIES),
  );
  const [loading, setLoading] = useState(false);
  const [decision, setDecision] = useState<ProductTrustDecision | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function load(signal?: AbortSignal) {
    if (!workspaceId) {
      setStatus(null);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const next = await client.getProjectTrust(workspaceId, { signal });
      setStatus(next);
      if (next.granted_capabilities.length > 0) {
        setSelected(new Set(next.granted_capabilities));
      }
    } catch (loadError) {
      if (!signal?.aborted) {
        setError(loadError instanceof Error ? loadError.message : String(loadError));
      }
    } finally {
      if (!signal?.aborted) {
        setLoading(false);
      }
    }
  }

  useEffect(() => {
    const controller = new AbortController();
    void load(controller.signal);
    return () => controller.abort();
  }, [client, workspaceId]);

  const selectedCapabilities = useMemo(
    () => PRODUCT_TRUST_CAPABILITIES.filter((capability) => selected.has(capability)),
    [selected],
  );

  async function decide(nextDecision: ProductTrustDecision) {
    if (!workspaceId || decision !== null) {
      return;
    }
    if (nextDecision === "grant" && selectedCapabilities.length === 0) {
      setError("Select at least one capability before granting trust.");
      return;
    }
    setDecision(nextDecision);
    setError(null);
    try {
      const next = await client.decideProjectTrust(workspaceId, {
        decision: nextDecision,
        capabilities: nextDecision === "grant" ? selectedCapabilities : [],
      });
      setStatus(next);
    } catch (decisionError) {
      setError(
        decisionError instanceof Error
          ? decisionError.message
          : String(decisionError),
      );
    } finally {
      setDecision(null);
    }
  }

  function toggleCapability(capability: ProductTrustCapability) {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(capability)) {
        next.delete(capability);
      } else {
        next.add(capability);
      }
      return next;
    });
  }

  const busy = loading || decision !== null;
  const stateLabel = status
    ? status.state === "unknown"
      ? "Unknown"
      : status.state === "restricted"
        ? "Restricted"
        : status.state === "trusted"
          ? "Trusted"
          : "Revoked"
    : "Unavailable";

  return (
    <div className="settings-card" aria-busy={busy}>
      <div className="settings-card__heading">
        <div>
          <h2>Project trust</h2>
          <p className="placeholder-note">
            {workspaceId
              ? `${stateLabel} · ${status?.identity_digest.slice(0, 18) ?? "loading"}`
              : "Select a workspace to manage its trust state."}
          </p>
        </div>
        {workspaceId ? (
          <button
            type="button"
            className="secondary"
            aria-label="Refresh project trust"
            title="Refresh project trust"
            disabled={busy}
            onClick={() => void load()}
          >
            <ReloadIcon />
          </button>
        ) : null}
      </div>

      {workspaceId ? (
        <>
          <fieldset className="trust-capabilities" disabled={busy}>
            <legend>Capabilities</legend>
            {PRODUCT_TRUST_CAPABILITIES.map((capability) => (
              <label key={capability}>
                <input
                  type="checkbox"
                  checked={selected.has(capability)}
                  onChange={() => toggleCapability(capability)}
                />
                <span>{CAPABILITY_LABELS[capability]}</span>
                {status?.invalidated_capabilities.includes(capability) ? (
                  <small>Changed</small>
                ) : status?.granted_capabilities.includes(capability) ? (
                  <small>Granted</small>
                ) : null}
              </label>
            ))}
          </fieldset>

          <div className="field-actions">
            <button
              type="button"
              disabled={busy || selectedCapabilities.length === 0}
              onClick={() => void decide("grant")}
            >
              <CheckIcon /> {decision === "grant" ? "Granting..." : "Grant selected"}
            </button>
            <button
              type="button"
              className="secondary"
              disabled={busy}
              onClick={() => void decide("deny")}
            >
              <Cross2Icon /> {decision === "deny" ? "Denying..." : "Deny"}
            </button>
            <button
              type="button"
              className="danger"
              disabled={busy}
              onClick={() => void decide("revoke")}
            >
              <LockClosedIcon /> {decision === "revoke" ? "Revoking..." : "Revoke"}
            </button>
          </div>
        </>
      ) : null}

      {error ? (
        <div className="chat-error" role="alert">
          {error}
        </div>
      ) : null}
    </div>
  );
}
