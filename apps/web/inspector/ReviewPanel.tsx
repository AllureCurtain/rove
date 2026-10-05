"use client";

import { MagnifyingGlassIcon, ReloadIcon, StopIcon } from "@radix-ui/react-icons";
import { useEffect, useRef } from "react";
import { useCopy } from "../copy/CopyProvider";
import type { createTranslator } from "../copy";

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
  const { t } = useCopy();
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
    <section className="review-panel" aria-label={t("review.title")} data-review-panel>
      <div className="inspector-section__heading">
        <h3>{t("review.title")}</h3>
        <button
          type="button"
          className="ghost icon-button"
          onClick={onRefresh}
          disabled={loading}
          aria-label={t("review.refresh")}
          title={t("review.refresh")}
        >
          <ReloadIcon />
        </button>
      </div>
      {error ? <p className="inspector-empty-line" role="alert">{t("review.error")}</p> : null}
      {loading && reviews.length === 0 ? (
        <p className="inspector-empty-line" role="status">{t("review.loading")}</p>
      ) : null}
      {reviews.length === 0 && !loading ? (
        <div className="inspector-state" data-tone="empty" role="status">
          <MagnifyingGlassIcon aria-hidden="true" />
          <strong>{t("review.empty")}</strong>
          <p>{t("review.emptyBody")}</p>
        </div>
      ) : null}
      {reviews.length > 0 ? (
        <>
          <label className="review-panel__select-label" htmlFor="review-run-select">
            {t("review.run")}
          </label>
          <select
            id="review-run-select"
            value={selectedReviewId ?? ""}
            onChange={(event) => onSelect(event.target.value)}
          >
            {reviews.map((review) => (
              <option key={review.id} value={review.id}>
                {targetLabel(review, t)} · {t(`review.status_${review.status}`)}
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
  const { t } = useCopy();
  const result = review.result;
  const statusTone = reviewStatusTone(review.status);
  const isActive = review.status === "queued" || review.status === "running";
  return (
    <div className="review-panel__details" data-review-status={review.status}>
      <div className="inspector-section__heading">
        <span data-tone={statusTone}>{t(`review.status_${review.status}`)}</span>
        {isActive ? (
          <button
            type="button"
            className="ghost icon-button"
            onClick={() => onCancel(review.id)}
            aria-label={t("review.cancel")}
            title={t("review.cancel")}
          >
            <StopIcon />
          </button>
        ) : null}
      </div>
      <dl className="inspector-facts">
        <div><dt>{t("review.target")}</dt><dd>{targetLabel(review, t)}</dd></div>
        <div><dt>{t("review.files")}</dt><dd>{review.target.entries}</dd></div>
        <div><dt>{t("review.findings")}</dt><dd>{review.findings_count}</dd></div>
        {review.unchecked_count > 0 ? (
          <div><dt>{t("review.unchecked")}</dt><dd>{review.unchecked_count}</dd></div>
        ) : null}
        {review.warnings_count > 0 ? (
          <div><dt>{t("review.warnings")}</dt><dd>{review.warnings_count}</dd></div>
        ) : null}
      </dl>
      {review.status === "pass" ? (
        <p className="review-panel__state" data-tone="ok">{t("review.passBody")}</p>
      ) : null}
      {review.findings_count > 0 ? (
        <div className="review-panel__findings">
          <strong>{t("review.findings")}</strong>
          {findings.length === 0 && findingsLoading ? (
            <p className="inspector-empty-line">{t("review.loadingFindings")}</p>
          ) : null}
          {findings.length === 0 && !findingsLoading ? (
            <p className="inspector-empty-line">{t("review.unavailableFindings")}</p>
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
                  {t("review.openFiles")}
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
              {findingsLoading ? t("common.loading") : t("review.more")}
            </button>
          ) : null}
        </div>
      ) : null}
      {review.status === "partial" ? (
        <p className="review-panel__state" data-tone="working">
          {t("review.partialBody")}
        </p>
      ) : null}
      {review.status === "stale" || review.status === "needs_attention" ? (
        <p className="review-panel__state" data-tone="error">
          {t("review.staleBody")}
        </p>
      ) : null}
      {review.status === "unavailable" ? (
        <p className="review-panel__state" data-tone="error">{t("review.unavailableBody")}</p>
      ) : null}
      {review.status === "cancelled" ? (
        <p className="review-panel__state">{t("review.cancelledBody")}</p>
      ) : null}
      {review.status === "error" ? (
        <p className="review-panel__state" data-tone="error">{t("review.errorBody")}</p>
      ) : null}
      {result?.warnings.length ? (
        <p className="inspector-empty-line">{t("review.warningBody")}</p>
      ) : null}
    </div>
  );
}

function targetLabel(review: ProductReview, t: ReturnType<typeof createTranslator>): string {
  const { spec } = review.target;
  if (spec.kind === "uncommitted") {
    return t("chat.reviewUncommitted");
  }
  return `${t(spec.kind === "base" ? "chat.reviewBase" : "chat.reviewCommit")}: ${spec.revision ?? t("review.unknown")}`;
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
