"use client";

import { useCallback, useEffect, useRef, useState } from "react";

import { describeError } from "../api/run-controller";
import type {
  ProductReview,
  ProductReviewFindingPageItem,
  ProductReviewFindingsResponse,
  ProductReviewTargetSpec,
} from "../product/product-api-types";
import type { ProductApiClient } from "../product/product-client";
import { newId } from "./product-types";

const REVIEW_POLL_INTERVAL_MS = 2_000;
const REVIEW_FINDING_PAGE_SIZE = 64;

export interface ProductReviewState {
  reviews: ProductReview[];
  selectedReviewId: string | null;
  selectedReview: ProductReview | null;
  findings: ProductReviewFindingPageItem[];
  findingsCursor: number | null;
  findingsLoading: boolean;
  loading: boolean;
  creating: boolean;
  error: string | null;
  refresh: () => Promise<boolean>;
  create: (target: ProductReviewTargetSpec) => Promise<boolean>;
  cancel: (reviewId: string) => Promise<boolean>;
  select: (reviewId: string) => void;
  loadFindings: (reviewId: string, cursor?: number) => Promise<boolean>;
}

export function useProductReviews({
  productClient,
  sessionId,
  workspaceKind,
}: {
  productClient: ProductApiClient;
  sessionId: string | null;
  workspaceKind?: "folder" | "repo";
}): ProductReviewState {
  const [reviews, setReviews] = useState<ProductReview[]>([]);
  const [selectedReviewId, setSelectedReviewId] = useState<string | null>(null);
  const [findings, setFindings] = useState<ProductReviewFindingPageItem[]>([]);
  const [findingsCursor, setFindingsCursor] = useState<number | null>(null);
  const [findingsLoading, setFindingsLoading] = useState(false);
  const [loading, setLoading] = useState(false);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const generationRef = useRef(0);
  const findingsGenerationRef = useRef(0);
  const focusedSessionRef = useRef<string | null>(sessionId);
  const selectedReviewIdRef = useRef<string | null>(null);

  useEffect(() => {
    focusedSessionRef.current = sessionId;
  }, [sessionId]);

  const refresh = useCallback(async (): Promise<boolean> => {
    const focusedSession = focusedSessionRef.current;
    const generation = ++generationRef.current;
    if (!focusedSession) {
      setReviews([]);
      selectedReviewIdRef.current = null;
      setSelectedReviewId(null);
      setFindings([]);
      setFindingsCursor(null);
      setLoading(false);
      return true;
    }
    setLoading(true);
    try {
      const response = await productClient.listReviews(focusedSession);
      const currentSelection = selectedReviewIdRef.current;
      const nextSelection =
        currentSelection &&
        response.reviews.some((review) => review.id === currentSelection)
          ? currentSelection
          : response.reviews[0]?.id ?? null;
      let nextReviews = response.reviews;
      if (nextSelection) {
        // The detail route performs the bounded authoritative stale check.
        // Hydrate only the selected Review so history listing never turns
        // into an unbounded sequence of Git captures.
        const selected = await productClient.getReview(nextSelection);
        nextReviews = response.reviews.map((review) =>
          review.id === selected.id ? selected : review,
        );
      }
      if (
        generationRef.current !== generation ||
        focusedSessionRef.current !== focusedSession
      ) {
        return false;
      }
      setReviews(nextReviews);
      selectedReviewIdRef.current = nextSelection;
      setSelectedReviewId(nextSelection);
      setError(null);
      return true;
    } catch (caught) {
      if (
        generationRef.current === generation &&
        focusedSessionRef.current === focusedSession
      ) {
        setError(`Could not load Reviews: ${describeError(caught)}`);
      }
      return false;
    } finally {
      if (generationRef.current === generation) {
        setLoading(false);
      }
    }
  }, [productClient]);

  useEffect(() => {
    ++generationRef.current;
    ++findingsGenerationRef.current;
    setReviews([]);
    selectedReviewIdRef.current = null;
    setSelectedReviewId(null);
    setFindings([]);
    setFindingsCursor(null);
    setError(null);
    void refresh();
    return () => {
      ++generationRef.current;
      ++findingsGenerationRef.current;
    };
  }, [refresh, sessionId]);

  useEffect(() => {
    if (!reviews.some((review) => review.status === "queued" || review.status === "running")) {
      return;
    }
    const interval = window.setInterval(() => {
      void refresh();
    }, REVIEW_POLL_INTERVAL_MS);
    return () => window.clearInterval(interval);
  }, [refresh, reviews]);

  const create = useCallback(
    async (target: ProductReviewTargetSpec): Promise<boolean> => {
      const focusedSession = focusedSessionRef.current;
      if (!focusedSession || workspaceKind !== "repo") {
        setError("Review requires a Git repository workspace.");
        return false;
      }
      setCreating(true);
      setError(null);
      try {
        const review = await productClient.createReview(focusedSession, {
          target,
          idempotency_key: newId("review"),
        });
        if (focusedSessionRef.current !== focusedSession) {
          return false;
        }
        setReviews((current) => [
          review,
          ...current.filter((item) => item.id !== review.id),
        ]);
        selectedReviewIdRef.current = review.id;
        setSelectedReviewId(review.id);
        setFindings([]);
        setFindingsCursor(null);
        return true;
      } catch (caught) {
        if (focusedSessionRef.current === focusedSession) {
          setError(`Could not start Review: ${describeError(caught)}`);
        }
        return false;
      } finally {
        setCreating(false);
      }
    },
    [productClient, workspaceKind],
  );

  const cancel = useCallback(
    async (reviewId: string): Promise<boolean> => {
      setError(null);
      try {
        const review = await productClient.cancelReview(reviewId);
        setReviews((current) =>
          current.map((item) => (item.id === review.id ? review : item)),
        );
        return true;
      } catch (caught) {
        setError(`Could not cancel Review: ${describeError(caught)}`);
        return false;
      }
    },
    [productClient],
  );

  const select = useCallback((reviewId: string) => {
    selectedReviewIdRef.current = reviewId;
    setSelectedReviewId(reviewId);
    setFindings([]);
    setFindingsCursor(null);
  }, []);

  const loadFindings = useCallback(
    async (reviewId: string, cursor?: number): Promise<boolean> => {
      const generation = ++findingsGenerationRef.current;
      setFindingsLoading(true);
      try {
        const response: ProductReviewFindingsResponse =
          await productClient.listReviewFindings(reviewId, {
            limit: REVIEW_FINDING_PAGE_SIZE,
            cursor,
          });
        if (findingsGenerationRef.current !== generation) {
          return false;
        }
        setFindings((current) =>
          cursor === undefined
            ? response.findings
            : [...current, ...response.findings],
        );
        setFindingsCursor(response.next_cursor ?? null);
        return true;
      } catch (caught) {
        if (findingsGenerationRef.current === generation) {
          setError(`Could not load Review findings: ${describeError(caught)}`);
        }
        return false;
      } finally {
        if (findingsGenerationRef.current === generation) {
          setFindingsLoading(false);
        }
      }
    },
    [productClient],
  );

  const selectedReview =
    reviews.find((review) => review.id === selectedReviewId) ?? null;

  return {
    reviews,
    selectedReviewId,
    selectedReview,
    findings,
    findingsCursor,
    findingsLoading,
    loading,
    creating,
    error,
    refresh,
    create,
    cancel,
    select,
    loadFindings,
  };
}
