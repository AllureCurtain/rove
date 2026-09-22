"use client";

import { FormEvent, type KeyboardEvent, type Ref, useState } from "react";
import {
  MagnifyingGlassIcon,
  PaperPlaneIcon,
  StopIcon,
} from "@radix-ui/react-icons";

import { useCopy } from "../copy/CopyProvider";
import type { ComposerDraftBinding } from "../state/composer-draft-store";
import { useComposerDraft } from "../state/use-composer-draft";
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
  draftBinding,
  disabled,
  busy,
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
  draftBinding?: ComposerDraftBinding;
  disabled: boolean;
  busy: boolean;
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
  const { t } = useCopy();
  const draft = useComposerDraft(draftBinding);
  const { text: message, submitting } = draft;
  const displayError = error || (draft.sendFailed ? t("chat.sendUnexpectedError") : null);
  const [reviewOpen, setReviewOpen] = useState(false);
  const [reviewKind, setReviewKind] = useState<ProductReviewTargetSpec["kind"]>("uncommitted");
  const [reviewRevision, setReviewRevision] = useState("");
  const canSubmit =
    Boolean(message.trim()) &&
    !submitting &&
    !disabled;

  async function submitDraft() {
    if (!canSubmit) return;
    await draft.submit(onSend);
  }

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    void submitDraft();
  }

  function handleKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key !== "Enter" || event.shiftKey) {
      return;
    }
    if (event.nativeEvent.isComposing || event.keyCode === 229) {
      return;
    }
    if (!event.ctrlKey && !event.metaKey) {
      return; // Plain Enter keeps the newline behavior.
    }
    event.preventDefault();
    if (event.repeat) return;
    void submitDraft();
  }

  return (
    <form className="chat-composer" onSubmit={handleSubmit} aria-label={t("chat.placeholder")}>
      {displayError ? (
        <div className="chat-error" id="composer-error" role="alert">
          {displayError}
        </div>
      ) : null}
      <div className="chat-composer__meta">
        {busy ? <span>{t("chat.responding")}</span> : null}
        {disabledReason ? <span>{disabledReason}</span> : null}
      </div>
      <div className="chat-composer__row">
        <textarea
          ref={textareaRef}
          aria-label={t("chat.placeholder")}
          aria-keyshortcuts="/ Control+Enter Meta+Enter"
          value={message}
          onChange={(event) => draft.setText(event.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={t("chat.placeholder")}
          disabled={disabled || submitting}
          aria-invalid={displayError ? "true" : undefined}
          aria-describedby={displayError ? "composer-error" : undefined}
        />
        <button type="submit" disabled={!canSubmit} aria-label={t("chat.send")}>
          <PaperPlaneIcon />
          {t("chat.send")}
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
          <span>{t("chat.disabledSettings")}</span>
        )}
        <div className="chat-composer__review">
          <button
            type="button"
            className="ghost"
            disabled={!reviewAvailable || reviewBusy}
            onClick={() => setReviewOpen((current) => !current)}
          >
            <MagnifyingGlassIcon aria-hidden="true" />
            {t("chat.review")}
          </button>
          {reviewOpen ? (
            <div className="chat-composer__review-form" data-review-launcher>
              <label htmlFor="review-target-kind">{t("chat.reviewTarget")}</label>
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
                <option value="uncommitted">{t("chat.reviewUncommitted")}</option>
                <option value="base">{t("chat.reviewBase")}</option>
                <option value="commit">{t("chat.reviewCommit")}</option>
              </select>
              {reviewKind !== "uncommitted" ? (
                <input
                  value={reviewRevision}
                  onChange={(event) => setReviewRevision(event.target.value)}
                  placeholder={
                    reviewKind === "base"
                      ? t("chat.reviewBasePlaceholder")
                      : t("chat.reviewCommitPlaceholder")
                  }
                  aria-label={t("chat.reviewRevision")}
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
                {reviewBusy ? t("chat.reviewStarting") : t("chat.reviewStart")}
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
  const { t } = useCopy();
  return (
    <button type="button" className="danger" onClick={onCancel} aria-label={t("chat.stop")}>
      <StopIcon />
      {t("chat.stop")}
    </button>
  );
}
