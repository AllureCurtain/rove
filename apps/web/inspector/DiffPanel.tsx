"use client";

import { ReloadIcon } from "@radix-ui/react-icons";
import { useCallback, useEffect, useMemo, useState } from "react";

import { createProductApiClient } from "../product/product-client";
import type { ProductDiffEntry } from "../product/product-api-types";

export function DiffPanel({ sessionId }: { sessionId: string }) {
  const client = useMemo(() => createProductApiClient(), []);
  const [entries, setEntries] = useState<ProductDiffEntry[]>([]);
  const [partial, setPartial] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const response = await client.getSessionDiff(sessionId, "all");
      setEntries(response.entries);
      setPartial(response.partial_reasons);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Failed to load diff");
    } finally {
      setLoading(false);
    }
  }, [client, sessionId]);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <section className="inspector-section" aria-label="Session diff">
      <div className="inspector-section__heading">
        <h3>Diff</h3>
        <button
          type="button"
          className="ghost icon-button"
          onClick={() => void load()}
          disabled={loading}
          aria-label="Refresh diff"
          title="Refresh diff"
        >
          <ReloadIcon />
        </button>
      </div>
      {error ? <p className="inspector-empty-line" role="alert">{error}</p> : null}
      {partial.length > 0 ? (
        <p className="inspector-empty-line">
          Partial: {partial[0]}{partial.length > 1 ? ` (+${partial.length - 1} more)` : ""}
        </p>
      ) : null}
      {entries.length === 0 && !error ? (
        <p className="inspector-empty-line">{loading ? "Loading diff…" : "No tool or Git changes recorded."}</p>
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
                    ? "binary"
                    : entry.truncated
                      ? "truncated"
                      : entry.reconstructable
                        ? "reconstructable"
                        : "summary only"}
                </small>
              </header>
              {entry.diff ? <pre className="evidence-diff-list__patch">{entry.diff}</pre> : (
                <p className="inspector-empty-line">No canonical patch was recorded.</p>
              )}
            </article>
          ))}
        </div>
      )}
    </section>
  );
}
