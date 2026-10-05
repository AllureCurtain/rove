"use client";

import {
  DownloadIcon,
  FileIcon,
  ImageIcon,
  ReloadIcon,
} from "@radix-ui/react-icons";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useCopy } from "../copy/CopyProvider";
import { createProductApiClient } from "../product/product-client";
import type {
  ProductArtifactContentEnvelope,
  ProductArtifactView,
} from "../product/product-api-types";

export function ArtifactPanel({ sessionId }: { sessionId: string }) {
  const { t } = useCopy();
  const client = useMemo(() => createProductApiClient(), []);
  const [artifacts, setArtifacts] = useState<ProductArtifactView[]>([]);
  const [partial, setPartial] = useState<string[]>([]);
  const [selected, setSelected] = useState<ProductArtifactView | null>(null);
  const [content, setContent] = useState<ProductArtifactContentEnvelope | null>(null);
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Request-generation guard (plan §3): artifact list and content responses may
  // only publish while their request is still the latest one for this panel, so
  // switching sessions cannot surface another session's artifacts.
  const requestRef = useRef(0);
  const downloadRequestRef = useRef(0);
  const previewRequestRef = useRef(0);
  useEffect(() => () => {
    requestRef.current += 1;
    downloadRequestRef.current += 1;
    previewRequestRef.current += 1;
  }, [sessionId]);

  useEffect(() => {
    return () => {
      if (previewUrl) URL.revokeObjectURL(previewUrl);
    };
  }, [previewUrl]);

  const load = useCallback(async () => {
    const request = ++requestRef.current;
    const stale = () => requestRef.current !== request;
    setLoading(true);
    setError(null);
    try {
      const response = await client.listSessionArtifacts(sessionId, true);
      if (!stale()) {
        setArtifacts(response.artifacts);
        setSelected((current) =>
          current
            ? response.artifacts.find((artifact) => artifact.artifact_id === current.artifact_id) ?? null
            : null,
        );
      }
    } catch (caught) {
      if (!stale()) {
        setError(caught instanceof Error ? caught.message : "Failed to load artifacts");
      }
    } finally {
      if (!stale()) {
        setLoading(false);
      }
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
    const request = ++requestRef.current;
    const stale = () => requestRef.current !== request;
    try {
      const nextContent = await client.getArtifactContent(sessionId, artifact.artifact_id);
      const nextPreviewUrl =
        nextContent.image && nextContent.preview_allowed
          ? URL.createObjectURL(
              await client.fetchArtifactPreview(sessionId, artifact.artifact_id),
            )
          : null;
      if (!stale()) {
        setContent(nextContent);
        setPreviewUrl(nextPreviewUrl);
      } else if (nextPreviewUrl) {
        URL.revokeObjectURL(nextPreviewUrl);
      }
    } catch (caught) {
      if (!stale()) {
        setError(caught instanceof Error ? caught.message : "Failed to open artifact");
      }
    }
    return () => { previewRequestRef.current += 1; };
  }

  async function downloadArtifact(artifact: ProductArtifactView) {
    const request = ++downloadRequestRef.current;
    const stale = () => downloadRequestRef.current !== request;
    setError(null);
    try {
      await downloadBlob(
        await client.fetchArtifactDownload(sessionId, artifact.artifact_id),
        artifact.safe_name,
      );
    } catch (caught) {
      if (!stale()) {
        setError(caught instanceof Error ? caught.message : "Failed to download artifact");
      }
    }
  }

  return (
    <section className="inspector-section" aria-label={t("inspector.artifacts")}>
      <div className="inspector-section__heading">
        <h3>{t("inspector.artifacts")}</h3>
        <button
          type="button"
          className="ghost icon-button"
          onClick={() => void load()}
          disabled={loading}
          aria-label={t("inspector.artifactsRefresh")}
          title={t("inspector.artifactsRefresh")}
        >
          <ReloadIcon />
        </button>
      </div>
      {error ? <p className="inspector-empty-line" role="alert">{t("chrome.loadError")}</p> : null}
      {loading && artifacts.length === 0 ? (
        <p className="inspector-empty-line">{t("inspector.artifactsLoading")}</p>
      ) : null}
      {partial.length > 0 ? (
        <p className="inspector-empty-line">
          {t("chrome.partial")}
        </p>
      ) : null}
      {!loading && artifacts.length === 0 ? (
        <p className="inspector-empty-line">{t("inspector.artifactsNone")}</p>
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
                  {t(`artifactLabels.${artifact.source_kind}`)} · {t(`artifactLabels.${artifact.availability}`)}
                  {artifact.size !== undefined ? ` · ${formatBytes(artifact.size)}` : ""}
                </small>
              </button>
              {artifact.availability === "available" || artifact.availability === "too_large" ? (
                <button
                  type="button"
                  className="ghost icon-button"
                  onClick={() => void downloadArtifact(artifact)}
                  aria-label={t("chrome.download", { name: artifact.safe_name })}
                  title={t("chrome.download", { name: artifact.safe_name })}
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
              <span>{selected.mime}</span>
            </div>
            {selected.availability === "available" || selected.availability === "too_large" ? (
              <button
                type="button"
                className="ghost icon-button"
                onClick={() => void downloadArtifact(selected)}
                aria-label={t("chrome.download", { name: selected.safe_name })}
                title={t("chrome.download", { name: selected.safe_name })}
              >
                <DownloadIcon />
              </button>
            ) : null}
          </div>
          {selected.sha256 ? (
            <p className="inspector-empty-line">SHA-256 <code>{selected.sha256}</code></p>
          ) : null}
          {selected.validation_error ? (
            <p className="inspector-empty-line" role="alert">{t("chrome.invalidContent")}</p>
          ) : null}
          {selected.availability === "cleaned" ? (
            <p className="inspector-empty-line">{t("chrome.cleaned")}</p>
          ) : null}
          {selected.availability === "invalid" ? (
            <p className="inspector-empty-line">{t("chrome.invalidContent")}</p>
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
            <p className="inspector-empty-line">{t("chrome.previewUnavailable")}</p>
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

function formatBytes(value: number): string {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MiB`;
}
