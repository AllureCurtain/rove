"use client";

import { useCallback, useEffect, useRef, useState } from "react";

import { useCopy } from "../copy/CopyProvider";
import type { ProductMessageSearchHit } from "../product/product-api-types";
import { ProductApiError } from "../product/product-client";
import {
  SESSION_SEARCH_PAGE_LIMIT,
  classifySessionSearchFailure,
  nextSessionSearchPage,
  sessionSearchTermError,
} from "./session-message-search-model";

export interface SessionMessageSearchPanelProps {
  open: boolean;
  sessionId: string | null;
  client: {
    searchSessionMessages: (
      sessionId: string,
      query: { q: string; cursor?: string; limit?: number },
    ) => Promise<{ hits: ProductMessageSearchHit[]; next_cursor?: string }>;
  };
  /** Locate a hit inside the open session; the answer is rendered verbatim. */
  onLocateMessage: (messageSeq: number) => "located" | "not_loaded";
  onClose: () => void;
}

type Status =
  | "idle"
  | "loading"
  | "ready"
  | "refused"
  | "not-found"
  | "error";

/**
 * Search this session's message ledger (R7 step 1).
 *
 * The endpoint's own rules are the ones a UI gets wrong quietly, so they are
 * enforced here: zero hits is only ever a successful empty page (a 400 refusal
 * and a 404 missing session get their own states), a cursor is only ever reused
 * with the term that produced it, and the returned excerpt is rendered verbatim
 * because it is already a bounded, redacted window.
 */
export function SessionMessageSearchPanel({
  open,
  sessionId,
  client,
  onLocateMessage,
  onClose,
}: SessionMessageSearchPanelProps) {
  const { t } = useCopy();
  const [term, setTerm] = useState("");
  const [issuedTerm, setIssuedTerm] = useState<string | null>(null);
  const [hits, setHits] = useState<ProductMessageSearchHit[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [status, setStatus] = useState<Status>("idle");
  const [inputError, setInputError] = useState<string | null>(null);
  const [errorCode, setErrorCode] = useState<string | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const [located, setLocated] = useState<{
    seq: number;
    result: "located" | "not_loaded";
  } | null>(null);
  const generationRef = useRef(0);
  const inputRef = useRef<HTMLInputElement>(null);

  // Another session's hits answer a different question; drop them with it.
  useEffect(() => {
    generationRef.current += 1;
    setHits([]);
    setNextCursor(null);
    setStatus("idle");
    setIssuedTerm(null);
    setInputError(null);
    setErrorCode(null);
    setLocated(null);
  }, [sessionId]);

  useEffect(() => {
    if (!open) {
      return;
    }
    const frame = window.requestAnimationFrame(() => inputRef.current?.focus());
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        onClose();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.cancelAnimationFrame(frame);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [open, onClose]);

  const run = useCallback(
    async (
      targetSession: string,
      query: { q: string; cursor?: string; limit?: number },
      append: boolean,
    ): Promise<void> => {
      const generation = generationRef.current + 1;
      generationRef.current = generation;
      if (append) {
        setLoadingMore(true);
      } else {
        setStatus("loading");
      }
      try {
        const page = await client.searchSessionMessages(targetSession, query);
        if (generationRef.current !== generation) {
          return;
        }
        setHits((current) => (append ? [...current, ...page.hits] : page.hits));
        setNextCursor(page.next_cursor ?? null);
        setStatus("ready");
        setErrorCode(null);
        setLocated(null);
      } catch (error) {
        if (generationRef.current !== generation) {
          return;
        }
        const failure = classifySessionSearchFailure(
          error instanceof ProductApiError ? error.code : null,
        );
        setHits([]);
        setNextCursor(null);
        setLocated(null);
        if (failure.kind === "refused") {
          setStatus("refused");
          return;
        }
        if (failure.kind === "not-found") {
          setStatus("not-found");
          return;
        }
        setErrorCode(failure.code);
        setStatus("error");
      } finally {
        if (generationRef.current === generation) {
          setLoadingMore(false);
        }
      }
    },
    [client],
  );

  function submit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmed = term.trim();
    const error = sessionSearchTermError(term);
    if (error !== null) {
      setInputError(error);
      return;
    }
    if (!sessionId) {
      setInputError("commandPalette.searchUnavailable");
      return;
    }
    setInputError(null);
    setHits([]);
    setNextCursor(null);
    setLocated(null);
    setIssuedTerm(trimmed);
    void run(sessionId, { q: trimmed, limit: SESSION_SEARCH_PAGE_LIMIT }, false);
  }

  function loadMore() {
    if (!sessionId || issuedTerm === null) {
      return;
    }
    const request = nextSessionSearchPage({
      currentTerm: term.trim(),
      issuedTerm,
      nextCursor,
    });
    if (request === null) {
      return;
    }
    void run(sessionId, request, true);
  }

  function locate(hit: ProductMessageSearchHit) {
    setLocated({ seq: hit.message_seq, result: onLocateMessage(hit.message_seq) });
  }

  if (!open) {
    return null;
  }

  const exhausted = status === "ready" && nextCursor === null;

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
        aria-label={t("commandPalette.actionSearchSessionMessages")}
      >
        <header className="product-search__header">
          <div>
            <h2>{t("commandPalette.actionSearchSessionMessages")}</h2>
          </div>
          <button type="button" className="ghost" onClick={onClose}>
            {t("search.close")}
          </button>
        </header>
        <form className="product-search__form" onSubmit={submit}>
          <input
            ref={inputRef}
            id="session-message-search-term"
            aria-label={t("commandPalette.actionSearchSessionMessages")}
            placeholder={t("search.placeholder")}
            value={term}
            onChange={(event) => setTerm(event.target.value)}
          />
          <button type="submit" disabled={status === "loading"}>
            {t("search.submit")}
          </button>
        </form>

        {!sessionId ? (
          <p role="status">{t("commandPalette.searchUnavailable")}</p>
        ) : null}
        {inputError ? (
          <p className="product-search__error" role="alert">
            {t(inputError)}
          </p>
        ) : null}
        {status === "loading" ? <p role="status">{t("commandPalette.searchLoading")}</p> : null}
        {status === "refused" ? (
          // The endpoint refused what was asked (usually a cursor that no longer
          // matches the term). That is not an empty result.
          <p className="product-search__error" role="alert">
            {t("commandPalette.searchError", { error: "product_invalid_input" })}
          </p>
        ) : null}
        {status === "not-found" ? (
          <p className="product-search__error" role="alert">
            {t("commandPalette.searchError", { error: "product_not_found" })}
          </p>
        ) : null}
        {status === "error" ? (
          <p className="product-search__error" role="alert">
            {t("commandPalette.searchError", { error: errorCode ?? t("search.unknownError") })}
          </p>
        ) : null}
        {status === "ready" && hits.length === 0 ? (
          <p role="status">{t("commandPalette.searchEmpty")}</p>
        ) : null}

        {hits.length > 0 ? (
          <ul className="product-search__results">
            {hits.map((hit) => (
              <li key={hit.message_seq} className="product-search__hit">
                <div className="product-search__hit-meta">
                  <span>#{hit.message_seq}</span>
                  <span>{hit.created_at}</span>
                </div>
                {/* Already a bounded, redacted window: shown exactly as sent. */}
                <p className="product-search__snippet">{hit.snippet}</p>
                <button type="button" onClick={() => locate(hit)}>
                  {t("search.locateMessage")}
                </button>
                {located?.seq === hit.message_seq ? (
                  <p role="status">
                    {t(located.result === "located" ? "search.located" : "search.notLoaded")}
                  </p>
                ) : null}
              </li>
            ))}
          </ul>
        ) : null}

        <footer className="product-search__footer">
          {nextCursor !== null ? (
            <button type="button" onClick={loadMore} disabled={loadingMore}>
              {loadingMore ? t("search.loadingMore") : t("search.loadMore")}
            </button>
          ) : exhausted ? (
            <span>{t("search.exhausted")}</span>
          ) : null}
        </footer>
      </div>
    </div>
  );
}
