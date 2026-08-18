"use client";

import { MagnifyingGlassIcon, ReloadIcon, StopIcon } from "@radix-ui/react-icons";
import { useEffect, useRef } from "react";

import type {
  ProductReview,
  ProductReviewFindingPageItem,
} from "../product/product-api-types";

export function ReviewPanel({
  reviews,
  selectedReviewId,
  selectedReview,
  findings,
  findingsCursor,
  findingsLoading,
  loading,
  error,
  onSelect,
  onRefresh,
  onCancel,
  onLoadFindings,
  onOpenFinding,
}: {
  reviews: ProductReview[];
  selectedReviewId: string | null;
  selectedReview: ProductReview | null;
  findings: ProductReviewFindingPageItem[];
  findingsCursor: number | null;
  findingsLoading: boolean;
  loading: boolean;
  error: string | null;
  onSelect: (reviewId: string) => void;
  onRefresh: () => void;
  onCancel: (reviewId: string) => void;
  onLoadFindings: (reviewId: string, cursor?: number) => void;
  onOpenFinding: (path: string, line: number) => void;
}) {
  const requestedReviewIdRef = useRef<string | null>(null);
  useEffect(() => {
    if (
      selectedReview &&
      selectedReview.findings_count > 0 &&
      findings.length === 0 &&
      !findingsLoading &&
      requestedReviewIdRef.current !== selectedReview.id
    ) {
      requestedReviewIdRef.current = selectedReview.id;
      onLoadFindings(selectedReview.id);
    }
  }, [findings.length, findingsLoading, onLoadFindings, selectedReview]);

  useEffect(() => {
    if (requestedReviewIdRef.current !== selectedReviewId) {
      requestedReviewIdRef.current = null;
    }
  }, [selectedReviewId]);

  return (
    <section className="review-panel" aria-label="Read-only Review" data-review-panel>
      <div className="inspector-section__heading">
        <h3>Read-only Review</h3>
        <button
          type="button"
          className="ghost icon-button"
          onClick={onRefresh}
          disabled={loading}
          aria-label="Refresh Reviews"
          title="Refresh Reviews"
        >
          <ReloadIcon />
        </button>
      </div>
      {error ? <p className="inspector-empty-line" role="alert">{error}</p> : null}
      {loading && reviews.length === 0 ? (
        <p className="inspector-empty-line" role="status">Loading Reviews…</p>
      ) : null}
      {reviews.length === 0 && !loading ? (
        <div className="inspector-state" data-tone="empty" role="status">
          <MagnifyingGlassIcon aria-hidden="true" />
          <strong>No Review runs</strong>
          <p>Start a Review from the composer to inspect a Git target without changing it.</p>
        </div>
      ) : null}
      {reviews.length > 0 ? (
        <>
          <label className="review-panel__select-label" htmlFor="review-run-select">
            Review run
          </label>
          <select
            id="review-run-select"
            value={selectedReviewId ?? ""}
            onChange={(event) => onSelect(event.target.value)}
          >
            {reviews.map((review) => (
              <option key={review.id} value={review.id}>
                {targetLabel(review)} · {statusLabel(review.status)}
              </option>
            ))}
          </select>
          {selectedReview ? (
            <ReviewDetails
              review={selectedReview}
              findings={findings}
              findingsCursor={findingsCursor}
              findingsLoading={findingsLoading}
              onCancel={onCancel}
              onLoadFindings={onLoadFindings}
              onOpenFinding={onOpenFinding}
            />
          ) : null}
        </>
      ) : null}
    </section>
  );
}

function ReviewDetails({
  review,
  findings,
  findingsCursor,
  findingsLoading,
  onCancel,
  onLoadFindings,
  onOpenFinding,
}: {
  review: ProductReview;
  findings: ProductReviewFindingPageItem[];
  findingsCursor: number | null;
  findingsLoading: boolean;
  onCancel: (reviewId: string) => void;
  onLoadFindings: (reviewId: string, cursor?: number) => void;
  onOpenFinding: (path: string, line: number) => void;
}) {
  const result = review.result;
  const statusTone = reviewStatusTone(review.status);
  const isActive = review.status === "queued" || review.status === "running";
  return (
    <div className="review-panel__details" data-review-status={review.status}>
      <div className="inspector-section__heading">
        <span data-tone={statusTone}>{statusLabel(review.status)}</span>
        {isActive ? (
          <button
            type="button"
            className="ghost icon-button"
            onClick={() => onCancel(review.id)}
            aria-label="Cancel Review"
            title="Cancel Review"
          >
            <StopIcon />
          </button>
        ) : null}
      </div>
      <dl className="inspector-facts">
        <div><dt>Target</dt><dd>{targetLabel(review)}</dd></div>
        <div><dt>Files</dt><dd>{review.target.entries}</dd></div>
        <div><dt>Findings</dt><dd>{review.findings_count}</dd></div>
        {review.unchecked_count > 0 ? (
          <div><dt>Unchecked</dt><dd>{review.unchecked_count}</dd></div>
        ) : null}
        {review.warnings_count > 0 ? (
          <div><dt>Warnings</dt><dd>{review.warnings_count}</dd></div>
        ) : null}
      </dl>
      {review.status === "pass" ? (
        <p className="review-panel__state" data-tone="ok">No actionable findings were reported.</p>
      ) : null}
      {review.findings_count > 0 ? (
        <div className="review-panel__findings">
          <strong>Findings</strong>
          {findings.length === 0 && findingsLoading ? (
            <p className="inspector-empty-line">Loading findings…</p>
          ) : null}
          {findings.length === 0 && !findingsLoading ? (
            <p className="inspector-empty-line">Finding details are unavailable.</p>
          ) : null}
          <ul className="review-finding-list">
            {findings.map(({ finding }) => (
              <li key={finding.finding_id} data-severity={finding.severity}>
                <div className="review-finding-list__heading">
                  <strong>{finding.title}</strong>
                  <span>{finding.severity}</span>
                </div>
                <p>{finding.path}:{finding.location.start_line || 1}</p>
                <p>{finding.explanation}</p>
                <button
                  type="button"
                  className="ghost"
                  onClick={() => onOpenFinding(finding.path, finding.location.start_line || 1)}
                >
                  Open in Files
                </button>
              </li>
            ))}
          </ul>
          {findingsCursor !== null ? (
            <button
              type="button"
              className="ghost"
              disabled={findingsLoading}
              onClick={() => onLoadFindings(review.id, findingsCursor)}
            >
              {findingsLoading ? "Loading…" : "Load more findings"}
            </button>
          ) : null}
        </div>
      ) : null}
      {review.status === "partial" ? (
        <p className="review-panel__state" data-tone="working">
          Review completed with bounded or unchecked portions. Inspect warnings before acting.
        </p>
      ) : null}
      {review.status === "stale" || review.status === "needs_attention" ? (
        <p className="review-panel__state" data-tone="error">
          The target changed or needs attention. Start a new Review for the current files.
        </p>
      ) : null}
      {review.status === "unavailable" ? (
        <p className="review-panel__state" data-tone="error">The Review target or runtime is unavailable.</p>
      ) : null}
      {review.status === "cancelled" ? (
        <p className="review-panel__state">Review was cancelled before completion.</p>
      ) : null}
      {review.status === "error" ? (
        <p className="review-panel__state" data-tone="error">Review runtime failed. No chat turn was changed.</p>
      ) : null}
      {result?.warnings.length ? (
        <p className="inspector-empty-line">{result.warnings[0]}</p>
      ) : null}
    </div>
  );
}

function targetLabel(review: ProductReview): string {
  const { spec } = review.target;
  if (spec.kind === "uncommitted") {
    return "Uncommitted changes";
  }
  return `${spec.kind === "base" ? "Base" : "Commit"}: ${spec.revision ?? "unknown"}`;
}

function statusLabel(status: ProductReview["status"]): string {
  return status.replaceAll("_", " ");
}

function reviewStatusTone(status: ProductReview["status"]): "ok" | "working" | "error" | undefined {
  if (status === "pass") return "ok";
  if (status === "queued" || status === "running" || status === "partial") return "working";
  if (
    status === "stale" ||
    status === "needs_attention" ||
    status === "unavailable" ||
    status === "error"
  ) {
    return "error";
  }
  return undefined;
}
