"use client";

import { DownloadIcon } from "@radix-ui/react-icons";
import { useMemo, useState } from "react";

import { useCopy } from "../copy/CopyProvider";
import { createProductApiClient, type ProductExportFormat } from "../product/product-client";
import { downloadEvidenceFile } from "../product/evidence-export";

export function ExportPanel({ sessionId }: { sessionId: string }) {
  const { t } = useCopy();
  const client = useMemo(() => createProductApiClient(), []);
  const [format, setFormat] = useState<ProductExportFormat>("json");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function download(): Promise<void> {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      downloadEvidenceFile(await client.exportSessionEvidence(sessionId, format));
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : t("settings.export.title"));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="inspector-section" aria-label={t("settings.export.title")}>
      <h3>{t("settings.export.title")}</h3>
      <p className="inspector-empty-line">{t("settings.export.desc")}</p>
      <div className="evidence-export-controls">
        <label>
          <span>{t("chrome.format")}</span>
          <select
            value={format}
            disabled={busy}
            onChange={(event) => setFormat(event.target.value as ProductExportFormat)}
          >
            <option value="json">JSON</option>
            <option value="html">{t("chrome.offlineHtml")}</option>
            <option value="markdown">Markdown</option>
          </select>
        </label>
        <button type="button" onClick={() => void download()} disabled={busy}>
          <DownloadIcon /> {busy ? t("common.loading") : t("settings.export.download")}
        </button>
      </div>
      {error ? <p className="inspector-empty-line" role="alert">{t("chrome.loadError")}</p> : null}
    </section>
  );
}
