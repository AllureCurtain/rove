"use client";

import { ReloadIcon } from "@radix-ui/react-icons";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useCopy } from "../copy/CopyProvider";
import { createProductApiClient } from "../product/product-client";
import type { ProductDiffEntry } from "../product/product-api-types";

export function DiffPanel({ sessionId }: { sessionId: string }) {
  const { t } = useCopy();
  const client = useMemo(() => createProductApiClient(), []);
  const [entries, setEntries] = useState<ProductDiffEntry[]>([]);
  const [partial, setPartial] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  // Request-generation guard (plan §3): a diff response may only publish while
  // its request is still the latest one for this panel, so switching sessions
  // cannot surface another session's evidence.
  const requestRef = useRef(0);

  const load = useCallback(async () => {
    const request = ++requestRef.current;
    const stale = () => requestRef.current !== request;
    setLoading(true);
    setError(null);
    try {
      const response = await client.getSessionDiff(sessionId, "all");
      if (!stale()) {
        setEntries(response.entries);
        setPartial(response.partial_reasons);
      }
    } catch (caught) {
      if (!stale()) {
        setError(caught instanceof Error ? caught.message : "Failed to load diff");
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

  return (
    <section className="inspector-section" aria-label={t("inspector.diff")}>
      <div className="inspector-section__heading">
        <h3>{t("inspector.diff")}</h3>
        <button
          type="button"
          className="ghost icon-button"
          onClick={() => void load()}
          disabled={loading}
          aria-label={t("chrome.refreshDiff")}
          title={t("chrome.refreshDiff")}
        >
          <ReloadIcon />
        </button>
      </div>
      {error ? <p className="inspector-empty-line" role="alert">{t("chrome.loadError")}</p> : null}
      {partial.length > 0 ? (
        <p className="inspector-empty-line">
          {t("chrome.partial")}
        </p>
      ) : null}
      {entries.length === 0 && !error ? (
        <p className="inspector-empty-line">{loading ? t("inspector.diffLoading") : t("inspector.diffNone")}</p>
      ) : (
        <div className="evidence-diff-list">
          {entries.map((entry, index) => (
            <article key={`${entry.source}:${entry.source_run_id ?? "workspace"}:${entry.op}:${entry.path}:${index}`}>
              <header>
                <div>
                  <strong>{entry.path}</strong>
                  <span>{entry.source} · {entry.op}</span>
                </div>
                <small data-tone={entry.reconstructable ? "ok" : "muted"}>
                  {entry.binary
                    ? t("chrome.binary")
                    : entry.truncated
                      ? t("chrome.truncated")
                      : entry.reconstructable
                        ? t("chrome.fullPatch")
                        : t("chrome.summaryOnly")}
                </small>
              </header>
              {entry.diff ? <pre className="evidence-diff-list__patch">{entry.diff}</pre> : (
                <p className="inspector-empty-line">{t("chrome.noPatch")}</p>
              )}
            </article>
          ))}
        </div>
      )}
    </section>
  );
}
