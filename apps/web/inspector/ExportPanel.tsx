"use client";

import { DownloadIcon } from "@radix-ui/react-icons";
import { useMemo, useState } from "react";

import { createProductApiClient, type ProductExportFormat } from "../product/product-client";
import { downloadEvidenceFile } from "../product/evidence-export";

export function ExportPanel({ sessionId }: { sessionId: string }) {
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
      setError(caught instanceof Error ? caught.message : "Evidence export failed");
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="inspector-section" aria-label="Session evidence export">
      <h3>Evidence export</h3>
      <div className="evidence-export-controls">
        <label>
          <span>Format</span>
          <select
            value={format}
            disabled={busy}
            onChange={(event) => setFormat(event.target.value as ProductExportFormat)}
          >
            <option value="json">JSON</option>
            <option value="html">Offline HTML</option>
            <option value="markdown">Markdown</option>
          </select>
        </label>
        <button type="button" onClick={() => void download()} disabled={busy}>
          <DownloadIcon /> {busy ? "Preparing..." : "Download"}
        </button>
      </div>
      {error ? <p className="inspector-empty-line" role="alert">{error}</p> : null}
    </section>
  );
}
