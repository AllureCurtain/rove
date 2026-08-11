"use client";

import {
  DownloadIcon,
  FileIcon,
  ImageIcon,
  ReloadIcon,
} from "@radix-ui/react-icons";
import { useCallback, useEffect, useMemo, useState } from "react";

import { createProductApiClient } from "../product/product-client";
import type {
  ProductArtifactContentEnvelope,
  ProductArtifactSourceKind,
  ProductArtifactView,
} from "../product/product-api-types";

// Reads as a label rather than a wire value, and distinguishes a durable Tool
// Artifact produced by a tool call from a file the run registered itself.
const SOURCE_KIND_LABELS: Record<ProductArtifactSourceKind, string> = {
  report: "report",
  task_state: "task state",
  trace: "trace",
  registered: "registered",
  tool_artifact: "tool artifact",
};

export function ArtifactPanel({ sessionId }: { sessionId: string }) {
  const client = useMemo(() => createProductApiClient(), []);
  const [artifacts, setArtifacts] = useState<ProductArtifactView[]>([]);
  const [partial, setPartial] = useState<string[]>([]);
  const [selected, setSelected] = useState<ProductArtifactView | null>(null);
  const [content, setContent] = useState<ProductArtifactContentEnvelope | null>(null);
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    return () => {
      if (previewUrl) URL.revokeObjectURL(previewUrl);
    };
  }, [previewUrl]);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const response = await client.listSessionArtifacts(sessionId, true);
      setArtifacts(response.artifacts);
      setPartial(response.partial_reasons);
      setSelected((current) =>
        current
          ? response.artifacts.find((artifact) => artifact.artifact_id === current.artifact_id) ?? null
          : null,
      );
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Failed to load artifacts");
    } finally {
      setLoading(false);
    }
  }, [client, sessionId]);

  useEffect(() => {
    void load();
  }, [load]);

  async function openArtifact(artifact: ProductArtifactView) {
    setSelected(artifact);
    setContent(null);
    setPreviewUrl(null);
    setError(null);
    if (artifact.availability !== "available" || artifact.preview_kind === "unavailable") {
      return;
    }
    try {
      const nextContent = await client.getArtifactContent(sessionId, artifact.artifact_id);
      const nextPreviewUrl =
        nextContent.image && nextContent.preview_allowed
          ? URL.createObjectURL(
              await client.fetchArtifactPreview(sessionId, artifact.artifact_id),
            )
          : null;
      setContent(nextContent);
      setPreviewUrl(nextPreviewUrl);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Failed to open artifact");
    }
  }

  async function downloadArtifact(artifact: ProductArtifactView) {
    setError(null);
    try {
      await downloadBlob(
        await client.fetchArtifactDownload(sessionId, artifact.artifact_id),
        artifact.safe_name,
      );
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Failed to download artifact");
    }
  }

  return (
    <section className="inspector-section" aria-label="Run artifacts">
      <div className="inspector-section__heading">
        <h3>Artifacts</h3>
        <button
          type="button"
          className="ghost icon-button"
          onClick={() => void load()}
          disabled={loading}
          aria-label="Refresh artifacts"
          title="Refresh artifacts"
        >
          <ReloadIcon />
        </button>
      </div>
      {error ? <p className="inspector-empty-line" role="alert">{error}</p> : null}
      {loading && artifacts.length === 0 ? (
        <p className="inspector-empty-line">Loading artifacts…</p>
      ) : null}
      {partial.length > 0 ? (
        <p className="inspector-empty-line">
          Partial manifest: {partial[0]}{partial.length > 1 ? ` (+${partial.length - 1} more)` : ""}
        </p>
      ) : null}
      {!loading && artifacts.length === 0 ? (
        <p className="inspector-empty-line">No artifacts recorded.</p>
      ) : (
        <ul className="evidence-file-list">
          {artifacts.map((artifact) => (
            <li key={artifact.artifact_id} data-availability={artifact.availability}>
              <button
                type="button"
                className="ghost evidence-file-list__open"
                onClick={() => void openArtifact(artifact)}
              >
                {artifact.preview_kind === "raster_image" ? <ImageIcon /> : <FileIcon />}
                <span>{artifact.safe_name}</span>
                <small>
                  {SOURCE_KIND_LABELS[artifact.source_kind]} · {artifact.availability}
                  {artifact.size !== undefined ? ` · ${formatBytes(artifact.size)}` : ""}
                </small>
              </button>
              {artifact.availability === "available" || artifact.availability === "too_large" ? (
                <button
                  type="button"
                  className="ghost icon-button"
                  onClick={() => void downloadArtifact(artifact)}
                  aria-label={`Download ${artifact.safe_name}`}
                  title={`Download ${artifact.safe_name}`}
                >
                  <DownloadIcon />
                </button>
              ) : null}
            </li>
          ))}
        </ul>
      )}
      {selected ? (
        <div className="evidence-preview">
          <div className="evidence-preview__heading">
            <div>
              <strong>{selected.safe_name}</strong>
              <span>{selected.mime} · run {shortId(selected.source_run_id)}</span>
            </div>
            {selected.availability === "available" || selected.availability === "too_large" ? (
              <button
                type="button"
                className="ghost icon-button"
                onClick={() => void downloadArtifact(selected)}
                aria-label={`Download ${selected.safe_name}`}
                title={`Download ${selected.safe_name}`}
              >
                <DownloadIcon />
              </button>
            ) : null}
          </div>
          {selected.sha256 ? (
            <p className="inspector-empty-line">SHA-256 <code>{selected.sha256}</code></p>
          ) : null}
          {selected.validation_error ? (
            <p className="inspector-empty-line" role="alert">{selected.validation_error}</p>
          ) : null}
          {selected.availability === "cleaned" ? (
            <p className="inspector-empty-line">Artifact data has been cleaned.</p>
          ) : null}
          {selected.availability === "invalid" ? (
            <p className="inspector-empty-line">Artifact metadata or content is invalid.</p>
          ) : null}
          {content?.text !== undefined ? (
            <pre className="evidence-preview__text">{content.text}</pre>
          ) : null}
          {content?.image && content.preview_allowed ? (
            <figure className="evidence-preview__image">
              <img
                src={previewUrl ?? undefined}
                alt={selected.safe_name}
              />
              <figcaption>
                <ImageIcon aria-hidden="true" /> {content.image.width} × {content.image.height} {content.image.format}
              </figcaption>
            </figure>
          ) : null}
          {content && content.text === undefined && !content.image && !selected.validation_error ? (
            <p className="inspector-empty-line">Preview unavailable for this artifact type.</p>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}

async function downloadBlob(blob: Blob, filename: string): Promise<void> {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  anchor.hidden = true;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 0);
}

function shortId(value: string): string {
  return value.length > 12 ? `${value.slice(0, 8)}…${value.slice(-4)}` : value;
}

function formatBytes(value: number): string {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MiB`;
}
