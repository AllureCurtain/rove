"use client";

import {
  FormEvent,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ClipboardEvent,
  type CompositionEvent,
  type KeyboardEvent,
  type Ref,
} from "react";
import {
  MagnifyingGlassIcon,
  PaperPlaneIcon,
  StopIcon,
} from "@radix-ui/react-icons";

import { useCopy } from "../copy/CopyProvider";
import type { ComposerDraftBinding } from "../state/composer-draft-store";
import { useComposerDraft } from "../state/use-composer-draft";
import {
  MAX_PASTE_ATTACHMENTS,
  MAX_PASTE_BYTES,
  codePointLength,
  composeMessageWithAttachments,
  decidePaste,
  plainPasteText,
} from "./composer-paste";
import {
  COMPOSER_MENU_LIMIT,
  detectTrigger,
  fileMentionInsert,
  type ComposerTrigger,
} from "./composer-trigger";
import type { ActivityPhase } from "./activity-phase";
import type { ContextUsage } from "./context-usage";
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

/** Slash command offered in the composer menu, sourced by the product shell. */
export interface ComposerCommand {
  id: string;
  label: string;
  run: () => void;
}

export interface ComposerFileSuggestion {
  path: string;
  directory: boolean;
}

/** The file source caps its own list; the menu only reports the cap. */
export const COMPOSER_FILE_SOURCE_LIMIT = 200;

interface PasteAttachment {
  id: string;
  text: string;
}

interface MenuItem {
  key: string;
  label: string;
  accept: () => void;
}

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
  activityPhase,
  contextUsage,
  commands,
  findFiles,
  queuedEditNotice,
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
  /** Waiting line above the input; idle renders nothing. */
  activityPhase?: ActivityPhase;
  contextUsage?: ContextUsage | null;
  commands?: ComposerCommand[];
  findFiles?: (query: string) => Promise<ComposerFileSuggestion[]>;
  queuedEditNotice?: string | null;
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
  const recallCursor = useRef(-1);
  // Paste attachments live in memory for this draft only. A session switch
  // starts with an empty set, matching the draft store's lifecycle.
  const [attachments, setAttachments] = useState<PasteAttachment[]>([]);
  const [pasteNotice, setPasteNotice] = useState<string | null>(null);
  const [trigger, setTrigger] = useState<ComposerTrigger>(null);
  const composingRef = useRef(false);
  const attachmentBytes = useMemo(
    () =>
      attachments.reduce(
        (total, attachment) => total + new TextEncoder().encode(attachment.text).length,
        0,
      ),
    [attachments],
  );

  useEffect(() => {
    setAttachments([]);
    setPasteNotice(null);
    setTrigger(null);
    // Attachments belong to one session's draft; they never follow a switch.
  }, [draftBinding?.productSessionId, draftBinding?.workspaceId]);

  const canSubmit =
    Boolean(message.trim()) &&
    !submitting &&
    !disabled;

  async function submitDraft() {
    if (!canSubmit) return;
    await draft.submit(async (sent) => {
      const accepted = await onSend(composeMessageWithAttachments(sent, attachments));
      if (accepted) {
        setAttachments([]);
        setPasteNotice(null);
      }
      return accepted;
    });
  }

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    void submitDraft();
  }

  function refreshTrigger(value: string, caret: number | null) {
    if (composingRef.current) {
      // During an IME composition the menu must not follow the pinyin.
      return;
    }
    setTrigger(detectTrigger(value, caret ?? value.length));
  }

  function handlePaste(event: ClipboardEvent<HTMLTextAreaElement>) {
    const text = plainPasteText(event.clipboardData.getData("text/plain"));
    if (!text) return;
    const decision = decidePaste(text, {
      count: attachments.length,
      bytes: attachmentBytes,
    });
    if (decision.kind === "insert") return;
    event.preventDefault();
    if (decision.kind === "reject") {
      setPasteNotice(
        decision.reason === "too-many"
          ? t("chat.pasteRejectedTooMany", { n: MAX_PASTE_ATTACHMENTS })
          : t("chat.pasteRejectedTooLarge", { n: Math.round(MAX_PASTE_BYTES / 1024) }),
      );
      return;
    }
    setAttachments((current) => [...current, { id: createAttachmentId(), text }]);
    setPasteNotice(null);
  }

  function handleCompositionStart(_event: CompositionEvent<HTMLTextAreaElement>) {
    composingRef.current = true;
  }

  function handleCompositionEnd(event: CompositionEvent<HTMLTextAreaElement>) {
    composingRef.current = false;
    refreshTrigger(event.currentTarget.value, event.currentTarget.selectionStart);
  }

  // Latest wins: only the newest file query may paint the menu.
  const [fileItems, setFileItems] = useState<ComposerFileSuggestion[]>([]);
  const [fileListCapped, setFileListCapped] = useState(false);
  const filesSequenceRef = useRef(0);
  const fileQuery = trigger?.kind === "file" ? trigger.query : null;
  useEffect(() => {
    if (!findFiles || fileQuery === null) {
      setFileItems([]);
      setFileListCapped(false);
      return;
    }
    const sequence = ++filesSequenceRef.current;
    let cancelled = false;
    findFiles(fileQuery)
      .then((items) => {
        if (cancelled || sequence !== filesSequenceRef.current) {
          return;
        }
        setFileItems(items);
        setFileListCapped(items.length >= COMPOSER_FILE_SOURCE_LIMIT);
      })
      .catch(() => {
        // A failed source shows an empty menu; typing keeps working.
        if (!cancelled && sequence === filesSequenceRef.current) {
          setFileItems([]);
          setFileListCapped(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [fileQuery, findFiles]);

  const menuItems = useMemo<MenuItem[]>(() => {
    if (!trigger) {
      return [];
    }
    if (trigger.kind === "slash") {
      const query = trigger.query.toLocaleLowerCase();
      return (commands ?? [])
        .filter((command) => command.label.toLocaleLowerCase().includes(query))
        .slice(0, COMPOSER_MENU_LIMIT)
        .map((command) => ({
          key: command.id,
          label: command.label,
          accept: () => {
            command.run();
            clearTriggerText();
          },
        }));
    }
    return fileItems.slice(0, COMPOSER_MENU_LIMIT).map((suggestion) => ({
      key: suggestion.path,
      label: suggestion.path,
      accept: () => {
        acceptFileSuggestion(suggestion);
      },
    }));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [trigger, commands, fileItems, message]);

  const [activeIndex, setActiveIndex] = useState(0);
  // A file trigger keeps its menu open even when the source returned nothing:
  // the empty note lives inside the menu, so the reader can tell "no match"
  // apart from "no trigger". A slash trigger with no matching command closes.
  const menuOpen =
    trigger !== null && (trigger.kind === "file" || menuItems.length > 0);
  useEffect(() => {
    setActiveIndex(0);
  }, [menuItems]);

  function clearTriggerText() {
    if (!trigger) {
      return;
    }
    const [start, end] = trigger.range;
    draft.setText(message.slice(0, start) + message.slice(end));
    setTrigger(null);
  }

  function acceptFileSuggestion(suggestion: ComposerFileSuggestion) {
    if (!trigger) {
      return;
    }
    const [start, end] = trigger.range;
    const insertion = fileMentionInsert(suggestion.path, suggestion.directory);
    draft.setText(message.slice(0, start) + insertion + message.slice(end));
    setTrigger(null);
  }

  function acceptActiveMenuItem() {
    menuItems[activeIndex]?.accept();
  }

  function handleKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (menuOpen && !event.nativeEvent.isComposing) {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        const delta = event.key === "ArrowDown" ? 1 : -1;
        setActiveIndex(
          (current) => (current + delta + menuItems.length) % menuItems.length,
        );
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        setTrigger(null);
        return;
      }
      // Tab must not move focus out while the menu offers a completion.
      if (event.key === "Tab") {
        event.preventDefault();
        acceptActiveMenuItem();
        return;
      }
      if (
        event.key === "Enter" &&
        !event.ctrlKey &&
        !event.metaKey &&
        !event.shiftKey
      ) {
        event.preventDefault();
        acceptActiveMenuItem();
        return;
      }
    }
    if ((event.key === "ArrowUp" || event.key === "ArrowDown") && message.trim() === "") {
      const recalled = draft.recall(event.key === "ArrowUp" ? "older" : "newer", recallCursor.current);
      if (recalled) {
        event.preventDefault();
        recallCursor.current = recalled.cursor;
        draft.setText(recalled.text);
      }
      return;
    }
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

  const phaseText = activityPhase ? activityPhaseCopy(activityPhase, t) : "";

  return (
    <form className="chat-composer" onSubmit={handleSubmit} aria-label={t("chat.placeholder")}>
      {displayError ? (
        <div className="chat-error" id="composer-error" role="alert">
          {displayError}
        </div>
      ) : null}
      {phaseText ? (
        <p className="chat-composer__phase" role="status" aria-live="polite">
          {phaseText}
        </p>
      ) : null}
      <div className="chat-composer__meta">
        {busy ? <span>{t("chat.responding")}</span> : null}
        {disabledReason ? <span>{disabledReason}</span> : null}
        {queuedEditNotice ? <span>{queuedEditNotice}</span> : null}
      </div>
      {attachments.length > 0 ? (
        <div className="chat-composer__attachments">
          {attachments.map((attachment) => (
            <div className="paste-chip" key={attachment.id}>
              <details>
                <summary>{t("chat.pasteChip", { count: codePointLength(attachment.text) })}</summary>
                <pre tabIndex={0}>{attachment.text}</pre>
              </details>
              <span className="paste-chip__preview">
                {attachment.text.slice(0, 40)}
                {attachment.text.length > 40 ? "…" : ""}
              </span>
              <button
                type="button"
                className="icon-button"
                aria-label={t("common.close")}
                onClick={() =>
                  setAttachments((current) => current.filter((item) => item.id !== attachment.id))
                }
              >
                ×
              </button>
            </div>
          ))}
        </div>
      ) : null}
      {pasteNotice ? <p className="chat-composer__paste-notice" role="status">{pasteNotice}</p> : null}
      <div className="chat-composer__row">
        <textarea
          ref={textareaRef}
          aria-label={t("chat.placeholder")}
          aria-keyshortcuts="/ Control+Enter Meta+Enter"
          aria-activedescendant={menuOpen ? `composer-menu-item-${activeIndex}` : undefined}
          value={message}
          onChange={(event) => {
            draft.setText(event.target.value);
            refreshTrigger(event.target.value, event.target.selectionStart);
          }}
          onPaste={handlePaste}
          onKeyDown={handleKeyDown}
          onCompositionStart={handleCompositionStart}
          onCompositionEnd={handleCompositionEnd}
          onSelect={(event) => {
            refreshTrigger(event.currentTarget.value, event.currentTarget.selectionStart);
          }}
          placeholder={t("chat.placeholder")}
          disabled={disabled || submitting}
          aria-invalid={displayError ? "true" : undefined}
          aria-describedby={displayError ? "composer-error" : undefined}
        />
        {menuOpen ? (
          <ul
            className="composer-menu"
            id="composer-menu"
            role="listbox"
            aria-label={trigger?.kind === "slash" ? t("chat.triggerCommands") : t("chat.triggerFiles")}
          >
            {menuItems.map((item, index) => (
              <li
                key={item.key}
                id={`composer-menu-item-${index}`}
                role="option"
                aria-selected={index === activeIndex}
                data-active={index === activeIndex || undefined}
                // PreventDefault keeps the caret (and focus) in the textarea.
                onMouseDown={(event) => {
                  event.preventDefault();
                  item.accept();
                }}
              >
                {item.label}
              </li>
            ))}
            {trigger?.kind === "file" && fileListCapped ? (
              <li className="composer-menu__note" role="presentation">
                {t("chat.triggerFilesLimited", { n: COMPOSER_FILE_SOURCE_LIMIT })}
              </li>
            ) : null}
            {trigger?.kind === "file" && fileItems.length === 0 ? (
              <li className="composer-menu__note" role="presentation">
                {t("chat.triggerNoFiles")}
              </li>
            ) : null}
          </ul>
        ) : null}
        <button type="submit" disabled={!canSubmit} aria-label={t("chat.send")}>
          <PaperPlaneIcon />
          {t("chat.send")}
        </button>
        {contextUsage && contextUsage.used > 0 ? <ContextUsageRing usage={contextUsage} /> : null}
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

const CONTEXT_USAGE_MODE_KEY = "rove.ui-context-usage-mode";

function ContextUsageRing({ usage }: { usage: ContextUsage }) {
  const { t } = useCopy();
  const [mode, setMode] = useState<"used" | "remaining">(() =>
    typeof window !== "undefined" &&
    window.localStorage.getItem(CONTEXT_USAGE_MODE_KEY) === "remaining"
      ? "remaining"
      : "used",
  );

  function toggleMode() {
    setMode((current) => {
      const next = current === "used" ? "remaining" : "used";
      window.localStorage.setItem(CONTEXT_USAGE_MODE_KEY, next);
      return next;
    });
  }

  const used = new Intl.NumberFormat("en-US").format(usage.used);
  const ariaLabel =
    usage.window === null
      ? t("chat.usageAriaNoWindow", { used })
      : t("chat.usageAriaUsed", {
          used,
          total: new Intl.NumberFormat("en-US").format(usage.window),
        });
  const display =
    mode === "remaining" && usage.window !== null
      ? t("chat.usageModeRemaining", {
          n: new Intl.NumberFormat("en-US").format(Math.max(0, usage.window - usage.used)),
        })
      : t("chat.usageModeUsed", { n: used });
  const cacheTitle =
    usage.cacheRatio !== null
      ? ` ${t("chat.usageCache", { percent: Math.round(usage.cacheRatio * 100) })}`
      : "";

  return (
    <button
      type="button"
      className="context-usage"
      data-tone={usage.tone}
      data-window={usage.window !== null ? "known" : "unknown"}
      onClick={toggleMode}
      aria-label={ariaLabel}
      title={`${ariaLabel}.${cacheTitle}`}
    >
      {usage.window !== null ? (
        <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
          <circle className="context-usage__track" cx="8" cy="8" r="6.5" fill="none" strokeWidth="2.5" />
          <circle
            className="context-usage__fill"
            cx="8"
            cy="8"
            r="6.5"
            fill="none"
            strokeWidth="2.5"
            strokeDasharray={`${Math.min(1, usage.ratio ?? 0) * 40.84} 40.84`}
            transform="rotate(-90 8 8)"
          />
        </svg>
      ) : null}
      <span className="context-usage__value">{display}</span>
    </button>
  );
}

function activityPhaseCopy(
  phase: ActivityPhase,
  t: (path: string, params?: Record<string, string | number>) => string,
): string {
  switch (phase.kind) {
    case "idle":
      return "";
    case "sending":
      return t("chat.phaseSending");
    case "waiting-model":
      return t("chat.phaseWaitingModel");
    case "compacting":
      return phase.degraded
        ? t("chat.phaseCompactingDegraded")
        : t("chat.phaseCompacting");
    case "tool":
      return t("chat.phaseTool", { name: phase.name });
    case "planning":
      return t("chat.phasePlanning");
    case "finalizing":
      return t("chat.phaseFinalizing");
    case "degraded":
      return t("chat.phaseDegraded", { summary: phase.summary });
  }
}

function createAttachmentId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `paste_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 12)}`;
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
