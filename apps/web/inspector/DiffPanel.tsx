"use client";

import { ReloadIcon } from "@radix-ui/react-icons";
import parseDiff from "parse-diff";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useCopy } from "../copy/CopyProvider";
import { createProductApiClient } from "../product/product-client";
import type { ProductDiffEntry } from "../product/product-api-types";
import { DiffView } from "../product-v2/DiffView";
import { InspectorEmpty } from "./InspectorEmpty";

/**
 * Per-file +/− counts for the review surface (design §8: the header reads
 * "N changes +a −d"). Entries without a patch contribute nothing; a patch that
 * cannot be parsed still counts its own literal +/− lines so a headerless
 * canonical diff does not silently zero out.
 */
function diffEntryStats(diff: string | undefined): { additions: number; deletions: number } {
  if (!diff) {
    return { additions: 0, deletions: 0 };
  }
  try {
    const files = parseDiff(diff);
    if (files.length > 0) {
      return {
        additions: files.reduce((sum, file) => sum + file.additions, 0),
        deletions: files.reduce((sum, file) => sum + file.deletions, 0),
      };
    }
  } catch {
    // Fall through to the literal count.
  }
  let additions = 0;
  let deletions = 0;
  for (const line of diff.split("\n")) {
    if (line.startsWith("+++") || line.startsWith("---")) {
      continue;
    }
    if (line.startsWith("+")) additions += 1;
    else if (line.startsWith("-")) deletions += 1;
  }
  return { additions, deletions };
}

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

  const stats = useMemo(() => {
    const perEntry = entries.map((entry) => diffEntryStats(entry.diff));
    return {
      perEntry,
      additions: perEntry.reduce((sum, entry) => sum + entry.additions, 0),
      deletions: perEntry.reduce((sum, entry) => sum + entry.deletions, 0),
    };
  }, [entries]);

  return (
    <section className="inspector-section" aria-label={t("inspector.diff")}>
      <div className="inspector-section__heading">
        <h3>{t("inspector.diff")}</h3>
        {entries.length > 0 ? (
          <span className="evidence-diff-summary" role="status">
            {t("inspector.diffSummary", {
              count: entries.length,
              additions: stats.additions,
              deletions: stats.deletions,
            })}
          </span>
        ) : null}
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
        loading ? (
          <p className="inspector-empty-line">{t("inspector.diffLoading")}</p>
        ) : (
          <InspectorEmpty
            title={t("inspector.diffNone")}
            body={t("inspector.diffNoneBody")}
          />
        )
      ) : (
        <div className="evidence-diff-list">
          {entries.map((entry, index) => {
            const entryStats = stats.perEntry[index] ?? { additions: 0, deletions: 0 };
            return (
              // The first card opens so the pane is never a wall of collapsed
              // rows; every other file starts folded (design §8).
              <details
                className="evidence-diff-card"
                key={`${entry.source}:${entry.source_run_id ?? "workspace"}:${entry.op}:${entry.path}:${index}`}
                open={index === 0}
              >
                <summary className="evidence-diff-card__summary">
                  <span className="evidence-diff-card__path">{entry.path}</span>
                  <span className="evidence-diff-card__stats">
                    +{entryStats.additions} −{entryStats.deletions}
                  </span>
                  <small data-tone={entry.reconstructable ? "ok" : "muted"}>
                    {entry.binary
                      ? t("chrome.binary")
                      : entry.truncated
                        ? t("chrome.truncated")
                        : entry.reconstructable
                          ? t("chrome.fullPatch")
                          : t("chrome.summaryOnly")}
                  </small>
                </summary>
                <div className="evidence-diff-card__body">
                  <p className="inspector-empty-line">{entry.source} · {entry.op}</p>
                  {entry.diff ? (
                    <DiffView diff={entry.diff} sourcePath={entry.path} label={entry.path} />
                  ) : (
                    <p className="inspector-empty-line">{t("chrome.noPatch")}</p>
                  )}
                </div>
              </details>
            );
          })}
        </div>
      )}
    </section>
  );
}
