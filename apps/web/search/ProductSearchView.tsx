"use client";

import { useCallback, useEffect, useRef, useState } from "react";

import { useCopy } from "../copy/CopyProvider";
import { ProductApiError } from "../product/product-client";
import type { ProductSearchHit, ProductSearchResponse } from "../product/product-api-types";
import {
  MAX_SEARCH_TERM_BYTES,
  SEARCH_PAGE_LIMIT,
  describeSearchHit,
  expiredRecovery,
  formatSearchScope,
  groupHitsBySession,
  mergeSearchPage,
  nextSearchPageRequest,
  searchPageRequest,
  searchPartial,
  searchScopeOptions,
  searchTermBytes,
  workspaceScopeTermOk,
  type ProductSearchScope,
  type ProductSearchScopeKind,
} from "./product-search-model";

export interface ProductSearchViewProps {
  open: boolean;
  sessionId: string | null;
  workspaceId: string | null;
  /** Titles for the sessions a hit can name; a missing one renders as unknown. */
  sessions: ReadonlyArray<{ id: string; title: string }>;
  client: {
    searchProduct: (query: {
      q: string;
      scope: string;
      cursor?: string;
      limit?: number;
    }) => Promise<ProductSearchResponse>;
  };
  onOpenSession: (sessionId: string) => void;
  /**
   * Locate a message hit inside the session that is already open. The answer is
   * rendered verbatim, so the view never claims to have scrolled somewhere it
   * could not.
   */
  onLocateMessage?: (messageSeq: number) => "located" | "not_loaded";
  onClose: () => void;
}

type SearchStatus = "idle" | "loading" | "ready" | "error" | "expired";

/**
 * The unified product search view (R7 step 2).
 *
 * Three honesty rules drive the rendering, because each one is a way a bounded
 * answer could be mistaken for a complete one:
 *
 * - a page with fewer hits than requested that still carries a cursor is a
 *   bounded read, and says so (`search.partialBudget`);
 * - a term the scope cannot answer (the workspace scope below three characters)
 *   is refused before the request, never rendered as "no results";
 * - an expired cursor is dropped and the search restarts from the first page,
 *   and an expired *first* page is reported as content that no longer exists
 *   rather than as an empty result.
 */
export function ProductSearchView({
  open,
  sessionId,
  workspaceId,
  sessions,
  client,
  onOpenSession,
  onLocateMessage,
  onClose,
}: ProductSearchViewProps) {
  const { t } = useCopy();
  const [term, setTerm] = useState("");
  const [scopeKind, setScopeKind] = useState<ProductSearchScopeKind>("session");
  const [submitted, setSubmitted] = useState<{ term: string; scope: ProductSearchScope } | null>(
    null,
  );
  const [hits, setHits] = useState<ProductSearchHit[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [partial, setPartial] = useState(false);
  const [status, setStatus] = useState<SearchStatus>("idle");
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [notice, setNotice] = useState<"expired" | null>(null);
  const [inputError, setInputError] = useState<string | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const [located, setLocated] = useState<{ key: string; result: "located" | "not_loaded" } | null>(
    null,
  );
  const generationRef = useRef(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const scopeOptions = searchScopeOptions({ sessionId, workspaceId });

  // A session or workspace switch changes what the scopes mean, so a page that
  // belongs to the previous one is dropped instead of shown under the new one.
  useEffect(() => {
    generationRef.current += 1;
    setSubmitted(null);
    setHits([]);
    setNextCursor(null);
    setPartial(false);
    setStatus("idle");
    setErrorMessage(null);
    setNotice(null);
    setLocated(null);
  }, [sessionId, workspaceId]);

  useEffect(() => {
    if (!open) {
      return;
    }
    const frame = window.requestAnimationFrame(() => inputRef.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, [open]);

  useEffect(() => {
    if (!open) {
      return;
    }
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        onClose();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  const execute = useCallback(
    async (
      searchTerm: string,
      scope: ProductSearchScope,
      cursor: string | null,
      /**
       * Set when this request is the first page of a search that already had a
       * cursor dropped. The restart is a fact the user must keep seeing after
       * the new page loads, so it survives the successful page instead of being
       * cleared with the previous state.
       */
      announceExpiry = false,
    ): Promise<void> => {
      const generation = generationRef.current + 1;
      generationRef.current = generation;
      const append = cursor !== null;
      if (append) {
        setLoadingMore(true);
      } else {
        setStatus("loading");
      }
      try {
        const request =
          cursor === null
            ? searchPageRequest(searchTerm, scope, SEARCH_PAGE_LIMIT)
            : {
                q: searchTerm,
                scope: formatSearchScope(scope),
                cursor,
                limit: SEARCH_PAGE_LIMIT,
              };
        const page = await client.searchProduct(request);
        if (generationRef.current !== generation) {
          return;
        }
        if (page.scope !== formatSearchScope(scope)) {
          // The page answers a different question than the one asked; showing it
          // under this scope would be a fabrication.
          setStatus("error");
          setErrorMessage("search.scopeMismatch");
          return;
        }
        setHits((current) =>
          append ? (mergeSearchPage({ scope, hits: current }, page) ?? current) : page.hits,
        );
        setNextCursor(page.next_cursor ?? null);
        setPartial(
          searchPartial({
            hitCount: page.hits.length,
            limit: SEARCH_PAGE_LIMIT,
            nextCursor: page.next_cursor,
          }),
        );
        setNotice(announceExpiry ? "expired" : null);
        setStatus("ready");
      } catch (error) {
        if (generationRef.current !== generation) {
          return;
        }
        if (error instanceof ProductApiError && error.code === "product_events_expired") {
          if (expiredRecovery({ hadCursor: append }) === "restart") {
            // The cursor's run is gone (or its trace was cleaned): discard it and
            // restart once from the first page, never reusing the same cursor.
            await execute(searchTerm, scope, null, true);
            return;
          }
          setHits([]);
          setNextCursor(null);
          setPartial(false);
          setNotice("expired");
          setStatus("expired");
          return;
        }
        setStatus("error");
        setErrorMessage(
          error instanceof ProductApiError ? error.code : "search.unknownError",
        );
      } finally {
        if (generationRef.current === generation) {
          setLoadingMore(false);
        }
      }
    },
    [client],
  );

  function activeScope(): { scope: ProductSearchScope | null; reasonCopyKey: string | null } {
    const option = scopeOptions.find((candidate) => candidate.kind === scopeKind);
    if (!option?.available) {
      return { scope: null, reasonCopyKey: option?.reasonCopyKey ?? "search.unknownError" };
    }
    const id = scopeKind === "workspace" ? workspaceId : sessionId;
    if (!id) {
      return { scope: null, reasonCopyKey: option.reasonCopyKey ?? "search.unknownError" };
    }
    return { scope: { kind: scopeKind, id }, reasonCopyKey: null };
  }

  function submit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmed = term.trim();
    if (trimmed.length === 0) {
      setInputError("search.blank");
      return;
    }
    if (searchTermBytes(trimmed) > MAX_SEARCH_TERM_BYTES) {
      setInputError("search.tooLong");
      return;
    }
    const { scope, reasonCopyKey } = activeScope();
    if (!scope) {
      setInputError(reasonCopyKey);
      return;
    }
    if (scope.kind === "workspace" && !workspaceScopeTermOk(trimmed)) {
      setInputError("search.shortTermWorkspace");
      return;
    }
    setInputError(null);
    setLocated(null);
    setHits([]);
    setNextCursor(null);
    setPartial(false);
    setSubmitted({ term: trimmed, scope });
    void execute(trimmed, scope, null);
  }

  function loadMore() {
    if (!submitted || nextCursor === null) {
      return;
    }
    const request = nextSearchPageRequest(
      { term: submitted.term, scope: submitted.scope, limit: SEARCH_PAGE_LIMIT },
      { scope: formatSearchScope(submitted.scope), next_cursor: nextCursor },
    );
    if (!request) {
      return;
    }
    void execute(submitted.term, submitted.scope, nextCursor);
  }

  function selectScope(kind: ProductSearchScopeKind) {
    setScopeKind(kind);
    setInputError(null);
  }

  function locate(hit: ProductSearchHit) {
    if (!onLocateMessage || hit.source !== "message") {
      return;
    }
    setLocated({ key: hitKey(hit), result: onLocateMessage(hit.seq) });
  }

  if (!open) {
    return null;
  }

  const groups = groupHitsBySession(hits);
  const scopeLabelKey: Record<ProductSearchScopeKind, string> = {
    session: "search.scopeSession",
    workspace: "search.scopeWorkspace",
    trace: "search.scopeTrace",
  };
  const emptyCopyKey =
    submitted?.scope.kind === "trace"
      ? "search.emptyTrace"
      : submitted?.scope.kind === "workspace"
        ? "search.emptyWorkspace"
        : "search.empty";

  return (
    <div className="product-search-layer">
      <button
        type="button"
        className="product-search-scrim"
        aria-label={t("search.close")}
        tabIndex={-1}
        onClick={onClose}
      />
      <div
        className="product-search"
        role="dialog"
        aria-modal="true"
        aria-label={t("search.title")}
      >
        <header className="product-search__header">
          <div>
            <h2>{t("search.title")}</h2>
            <p className="product-search__lede">{t("search.lede")}</p>
          </div>
          <button type="button" className="ghost" onClick={onClose}>
            {t("search.close")}
          </button>
        </header>
        <form className="product-search__form" onSubmit={submit}>
          <label htmlFor="product-search-scope">{t("search.scopeLabel")}</label>
          <select
            id="product-search-scope"
            value={scopeKind}
            onChange={(event) => selectScope(event.target.value as ProductSearchScopeKind)}
          >
            {scopeOptions.map((option) => (
              <option key={option.kind} value={option.kind} disabled={!option.available}>
                {t(scopeLabelKey[option.kind])}
              </option>
            ))}
          </select>
          <input
            ref={inputRef}
            id="product-search-term"
            aria-label={t("search.placeholder")}
            placeholder={t("search.placeholder")}
            value={term}
            onChange={(event) => setTerm(event.target.value)}
          />
          <button type="submit" disabled={status === "loading"}>
            {t("search.submit")}
          </button>
        </form>
        <p className="product-search__scope-note">
          {(() => {
            const option = scopeOptions.find((candidate) => candidate.kind === scopeKind);
            return option && !option.available && option.reasonCopyKey
              ? t(option.reasonCopyKey)
              : "";
          })()}
        </p>
        {inputError ? (
          <p className="product-search__error" role="alert">
            {t(inputError)}
          </p>
        ) : null}
        {notice === "expired" && status !== "expired" ? (
          <p className="product-search__notice" role="status">
            {t("search.expired")}
          </p>
        ) : null}

        <div className="product-search__results">
          {status === "idle" ? <p role="status">{t("search.idle")}</p> : null}
          {status === "loading" ? <p role="status">{t("search.loading")}</p> : null}
          {status === "error" ? (
            // A typed code from the API, or the "no reason given" sentence: the
            // view never invents a cause for a failure.
            <p className="product-search__error" role="alert">
              {errorMessage === "search.unknownError" || errorMessage === "search.scopeMismatch"
                ? t(errorMessage)
                : t("search.error", { error: errorMessage ?? "" })}
            </p>
          ) : null}
          {status === "expired" ? (
            <p className="product-search__error" role="alert">
              {t("search.expired")}
            </p>
          ) : null}
          {status === "ready" && hits.length === 0 ? (
            <p role="status">{t(emptyCopyKey)}</p>
          ) : null}
          {status === "ready" && partial ? (
            <p className="product-search__notice" role="status">
              {t("search.partialBudget")}
            </p>
          ) : null}
          {groups.map((group) => {
            const title =
              sessions.find((session) => session.id === group.sessionId)?.title ??
              t("search.sessionUnknown", { id: group.sessionId });
            return (
              <section key={group.sessionId} className="product-search__group">
                <h3>
                  {title}
                  <small>{t("search.sessionHits", { count: group.hits.length })}</small>
                </h3>
                <ul>
                  {group.hits.map((hit) => {
                    const facts = describeSearchHit(hit);
                    const key = hitKey(hit);
                    const canLocate =
                      facts.locatable &&
                      hit.session_id === sessionId &&
                      typeof onLocateMessage === "function";
                    return (
                      <li key={key} className="product-search__hit">
                        <div className="product-search__hit-meta">
                          <span>{t(facts.sourceCopyKey)}</span>
                          {facts.runOrdinal !== null ? (
                            <span>{t("search.traceRun", { ordinal: facts.runOrdinal })}</span>
                          ) : (
                            <span>{t("search.traceNoRun")}</span>
                          )}
                          <span>{facts.hasCreatedAt ? hit.created_at : t("search.noTime")}</span>
                        </div>
                        {/* The excerpt is already bounded and redacted by the API;
                            it is shown verbatim, never re-cut or re-highlighted. */}
                        <p className="product-search__snippet">{hit.snippet}</p>
                        <div className="product-search__hit-actions">
                          {canLocate ? (
                            <button type="button" onClick={() => locate(hit)}>
                              {t("search.locateMessage")}
                            </button>
                          ) : (
                            // A trace hit's `seq` is the trace record's own
                            // position, not a message, so it is never sent to
                            // the locator.
                            <button
                              type="button"
                              onClick={() => onOpenSession(hit.session_id)}
                            >
                              {t("search.openSession")}
                            </button>
                          )}
                        </div>
                        {located?.key === key ? (
                          <p className="product-search__locate" role="status">
                            {t(located.result === "located" ? "search.located" : "search.notLoaded")}
                          </p>
                        ) : null}
                      </li>
                    );
                  })}
                </ul>
              </section>
            );
          })}
        </div>
        <footer className="product-search__footer">
          {nextCursor !== null ? (
            <button type="button" onClick={loadMore} disabled={loadingMore}>
              {loadingMore ? t("search.loadingMore") : t("search.loadMore")}
            </button>
          ) : status === "ready" ? (
            <span>{t("search.exhausted")}</span>
          ) : null}
        </footer>
      </div>
    </div>
  );
}

function hitKey(hit: ProductSearchHit): string {
  return `${hit.source}:${hit.session_id}:${hit.seq}`;
}
