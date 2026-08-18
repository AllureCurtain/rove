"use client";

import { FormEvent, type Ref, useState } from "react";
import {
  MagnifyingGlassIcon,
  PaperPlaneIcon,
  StopIcon,
} from "@radix-ui/react-icons";

import { QuickModelControl } from "../product-v2/QuickModelControl";
import type {
  ProviderProfileRecord,
  SessionModelConfig,
  SessionModelConfigInput,
} from "../state/product-types";
import type {
  ProductProviderModelsResponse,
  ProductReviewTargetSpec,
} from "../product/product-api-types";

export function Composer({
  disabled,
  busy,
  resumeLabel,
  disabledReason,
  error,
  profiles,
  modelConfig,
  modelConfigSaving,
  textareaRef,
  onSend,
  onCancel,
  onLoadProviderModels,
  onModelConfigChange,
  controlError,
  reviewAvailable = false,
  reviewBusy = false,
  reviewError = null,
  onCreateReview,
}: {
  disabled: boolean;
  busy: boolean;
  resumeLabel: string;
  disabledReason?: string;
  error: string | null;
  profiles: ProviderProfileRecord[];
  modelConfig: SessionModelConfig | null;
  modelConfigSaving: boolean;
  textareaRef?: Ref<HTMLTextAreaElement>;
  onSend: (message: string) => Promise<boolean> | boolean;
  onCancel: () => void;
  onLoadProviderModels: (profileId: string) => Promise<ProductProviderModelsResponse>;
  onModelConfigChange: (config: SessionModelConfigInput) => Promise<boolean>;
  controlError: string | null;
  reviewAvailable?: boolean;
  reviewBusy?: boolean;
  reviewError?: string | null;
  onCreateReview?: (target: ProductReviewTargetSpec) => Promise<boolean>;
}) {
  const [message, setMessage] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [reviewOpen, setReviewOpen] = useState(false);
  const [reviewKind, setReviewKind] = useState<ProductReviewTargetSpec["kind"]>("uncommitted");
  const [reviewRevision, setReviewRevision] = useState("");
  const canSubmit =
    Boolean(message.trim()) &&
    !submitting &&
    !disabled;

  async function handleSubmit(event: FormEvent) {
    event.preventDefault();
    const trimmed = message.trim();
    if (!trimmed || !canSubmit) {
      return;
    }
    setSubmitting(true);
    try {
      if (await onSend(trimmed)) {
        setMessage("");
      }
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <form className="chat-composer" onSubmit={handleSubmit} aria-label="Message composer">
      {error ? (
        <div className="chat-error" id="composer-error" role="alert">
          {error}
        </div>
      ) : null}
      <div className="chat-composer__meta">
        <span>{resumeLabel}</span>
        {busy ? <span>Streaming…</span> : null}
        {disabledReason ? <span>{disabledReason}</span> : null}
      </div>
      <div className="chat-composer__row">
        <textarea
          ref={textareaRef}
          aria-label="Message"
          aria-keyshortcuts="/"
          value={message}
          onChange={(event) => setMessage(event.target.value)}
          placeholder="Message the agent..."
          disabled={disabled || submitting}
          aria-invalid={error ? "true" : undefined}
          aria-describedby={error ? "composer-error" : undefined}
        />
        <button type="submit" disabled={!canSubmit} aria-label="Send message">
          <PaperPlaneIcon />
          Send
        </button>
        {busy ? <StopRunButton onCancel={onCancel} /> : null}
      </div>
      {controlError ? <p className="control-queue__error" role="alert">{controlError}</p> : null}
      <div className="chat-composer__controls">
        {modelConfig ? (
          <QuickModelControl
            profiles={profiles}
            modelConfig={modelConfig}
            saving={modelConfigSaving}
            loadProviderModels={onLoadProviderModels}
            onModelConfigChange={onModelConfigChange}
          />
        ) : (
          <span>Loading session model settings...</span>
        )}
        <div className="chat-composer__review">
          <button
            type="button"
            className="ghost"
            disabled={!reviewAvailable || reviewBusy}
            onClick={() => setReviewOpen((current) => !current)}
            title={reviewAvailable ? "Start a hard read-only Review" : "Review requires a Git repository"}
          >
            <MagnifyingGlassIcon aria-hidden="true" />
            Review
          </button>
          {reviewOpen ? (
            <div className="chat-composer__review-form" data-review-launcher>
              <label htmlFor="review-target-kind">Target</label>
              <select
                id="review-target-kind"
                value={reviewKind}
                onChange={(event) => {
                  const next = event.target.value as ProductReviewTargetSpec["kind"];
                  setReviewKind(next);
                  if (next === "uncommitted") setReviewRevision("");
                }}
                disabled={reviewBusy}
              >
                <option value="uncommitted">Uncommitted changes</option>
                <option value="base">Base revision</option>
                <option value="commit">Commit</option>
              </select>
              {reviewKind !== "uncommitted" ? (
                <input
                  value={reviewRevision}
                  onChange={(event) => setReviewRevision(event.target.value)}
                  placeholder={reviewKind === "base" ? "Base ref, e.g. main" : "Commit SHA"}
                  aria-label="Review revision"
                  disabled={reviewBusy}
                />
              ) : null}
              <button
                type="button"
                className="secondary"
                disabled={
                  !reviewAvailable ||
                  reviewBusy ||
                  (reviewKind !== "uncommitted" && !reviewRevision.trim())
                }
                onClick={() => {
                  if (!onCreateReview) return;
                  const target: ProductReviewTargetSpec =
                    reviewKind === "uncommitted"
                      ? { kind: "uncommitted" }
                      : { kind: reviewKind, revision: reviewRevision.trim() };
                  void onCreateReview(target).then((created) => {
                    if (created) {
                      setReviewOpen(false);
                      setReviewRevision("");
                    }
                  });
                }}
              >
                {reviewBusy ? "Starting…" : "Start Review"}
              </button>
              {reviewError ? <span className="chat-error" role="alert">{reviewError}</span> : null}
            </div>
          ) : null}
        </div>
      </div>
    </form>
  );
}

function StopRunButton({ onCancel }: { onCancel: () => void }) {
  return (
    <button type="button" className="danger" onClick={onCancel} aria-label="Stop run">
      <StopIcon />
      Stop
    </button>
  );
}
