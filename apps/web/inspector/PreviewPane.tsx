"use client";

import { ExternalLinkIcon, GlobeIcon, ReloadIcon } from "@radix-ui/react-icons";
import { useEffect, useMemo, useRef, useState } from "react";

import { useCopy } from "../copy/CopyProvider";
import { createProductApiClient } from "../product/product-client";
import type { ProductPreviewSession } from "../product/product-api-types";
import { InspectorEmpty } from "./InspectorEmpty";

/**
 * One workspace HTML preview (issue #28, design §8): a `preview` tab mounts
 * this with the workspace-relative entry path it owns. The pane asks the API
 * for an isolated preview session — a loopback origin whose token never
 * reaches product state, logs, or the transcript — and frames that origin, so
 * the workspace page executes without any path back into product credentials.
 * Third-party `http(s)` URLs are deliberately out of scope: sites answering
 * `X-Frame-Options` cannot be framed, so only workspace previews (and the
 * same-origin isolated origin they resolve to) are honored here.
 */
export function PreviewPane({
  workspaceId,
  path,
}: {
  workspaceId: string;
  path: string;
}) {
  const { t } = useCopy();
  const client = useMemo(() => createProductApiClient(), []);
  const [session, setSession] = useState<ProductPreviewSession | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [reloadKey, setReloadKey] = useState(0);
  const requestRef = useRef(0);

  // Each opened tab owns one preview session. Unmounting — the tab closing,
  // the workspace switching — revokes the token so the isolated origin stops
  // resolving it (A8). The session survives the iframe's own reloads: the
  // reload affordance only re-points the frame at the same session URL.
  useEffect(() => {
    const request = ++requestRef.current;
    const stale = () => requestRef.current !== request;
    let opened: ProductPreviewSession | null = null;
    async function load() {
      setLoading(true);
      setError(null);
      try {
        const next = await client.createWorkspacePreview(workspaceId, { path });
        if (next.workspace_id !== workspaceId) {
          throw new Error("Preview session does not match this workspace.");
        }
        if (!stale()) {
          opened = next;
          setSession(next);
        } else {
          void client
            .closeWorkspacePreview(next.workspace_id, next.preview_id)
            .catch(() => undefined);
        }
      } catch (caught) {
        if (!stale()) {
          setError(caught instanceof Error ? caught.message : "Failed to open preview");
        }
      } finally {
        if (!stale()) {
          setLoading(false);
        }
      }
    }
    void load();
    return () => {
      requestRef.current += 1;
      if (opened) {
        void client
          .closeWorkspacePreview(opened.workspace_id, opened.preview_id)
          .catch(() => undefined);
      }
    };
  }, [client, workspaceId, path]);

  const fileName = path.split("/").filter(Boolean).pop() ?? path;

  function openExternally() {
    if (!session) {
      return;
    }
    // The session URL already isolates the page on a separate origin; opening
    // it externally hands the same token to a top-level tab, which is exactly
    // what the URL was issued for (A7). It is never logged or persisted here.
    window.open(session.url, "_blank", "noopener,noreferrer");
  }

  return (
    <section className="inspector-section preview-pane" aria-label={fileName}>
      <div className="evidence-preview__heading preview-pane__heading">
        <div>
          <strong>{path}</strong>
          <span>{t("inspector.previewIsolated")}</span>
        </div>
        <button
          type="button"
          className="ghost icon-button"
          onClick={() => setReloadKey((value) => value + 1)}
          aria-label={t("inspector.previewReload")}
          title={t("inspector.previewReload")}
          disabled={!session}
        >
          <ReloadIcon />
        </button>
        <button
          type="button"
          className="ghost icon-button"
          onClick={openExternally}
          aria-label={t("inspector.previewOpenExternal")}
          title={t("inspector.previewOpenExternal")}
          disabled={!session}
        >
          <ExternalLinkIcon />
        </button>
      </div>
      {loading ? (
        <p className="inspector-empty-line">{t("inspector.previewLoading")}</p>
      ) : null}
      {error ? (
        <p className="inspector-empty-line" role="alert">
          {error}
        </p>
      ) : null}
      {!loading && !error && !session ? (
        <InspectorEmpty
          icon={<GlobeIcon />}
          title={t("inspector.previewUnavailable")}
          body={t("inspector.previewUnavailableBody")}
        />
      ) : null}
      {session ? (
        <iframe
          key={`${session.preview_id}:${reloadKey}`}
          className="preview-pane__frame"
          src={session.url}
          title={t("inspector.previewFrameTitle", { name: fileName })}
          // The preview origin is already untrusted and token-gated; sandboxing
          // keeps scripts and same-origin reads confined to that origin.
          sandbox="allow-scripts allow-same-origin allow-forms"
        />
      ) : null}
    </section>
  );
}
