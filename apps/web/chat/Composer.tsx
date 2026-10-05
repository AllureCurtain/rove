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
  PlusIcon,
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
import {
  ATTACHMENT_ACCEPT,
  MAX_FILE_ATTACHMENTS,
  attachmentByteCap,
  decideFileSelection,
  failureCopyKey,
  formatAttachmentBytes,
  hasUnfinishedFiles,
  isRetryableFailure,
  probeFailureFromError,
  requestAttachmentsForSend,
  rowFromUpload,
  rowsFromSendSnapshot,
  sendReferencesFromRows,
  uploadFailureFromError,
  warningCopyKey,
  type ComposerFileFailure,
  type ComposerFileRow,
} from "./composer-files";
import type { ActivityPhase } from "./activity-phase";
import type { ContextUsage } from "./context-usage";
import { QuickModelControl } from "../product-v2/QuickModelControl";
import type {
  ProviderProfileRecord,
  SessionModelConfig,
  SessionModelConfigInput,
} from "../state/product-types";
import type {
  ProductAttachmentUpload,
  ProductMessageAttachmentRequest,
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
  sendPaused = false,
  busy,
  disabledReason,
  error,
  profiles,
  modelConfig,
  modelConfigSaving,
  textareaRef,
  onSend,
  onUploadAttachment,
  onProbeAttachment,
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
  /**
   * A transcript re-read is in flight for the session on screen.
   *
   * The send waits for it, but the composer does not: the reader keeps typing,
   * keeps attaching, and a send pressed in this window is deferred until the
   * read lands instead of being swallowed. Locking the whole surface here is what
   * made a `Control+Enter` right after a stop do nothing — the disabled textarea
   * never even received the keystroke.
   */
  sendPaused?: boolean;
  busy: boolean;
  disabledReason?: string;
  error: string | null;
  profiles: ProviderProfileRecord[];
  modelConfig: SessionModelConfig | null;
  modelConfigSaving: boolean;
  textareaRef?: Ref<HTMLTextAreaElement>;
  onSend: (
    message: string,
    attachments: ProductMessageAttachmentRequest[],
  ) => Promise<boolean> | boolean;
  /**
   * Uploads one file for the active session (R8). Absent disables file
   * attachments entirely — the composer then keeps only its paste chips.
   */
  onUploadAttachment?: (file: File, name: string) => Promise<ProductAttachmentUpload>;
  /**
   * Asks the server whether a restored reference is still readable. Rejects
   * with a typed `ProductApiError` when it is not, and the row says so instead
   * of carrying a dead id into the next send.
   */
  onProbeAttachment?: (attachmentId: string) => Promise<void>;
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
  const [files, setFiles] = useState<ComposerFileRow[]>([]);
  const [fileNotice, setFileNotice] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  // Bytes for a row that failed and may be retried. Cleared the moment a row
  // succeeds or leaves, so a 20 MiB file is never held after it is uploaded.
  const fileBlobsRef = useRef(new Map<string, File>());
  const [dropActive, setDropActive] = useState(false);
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
    setFiles([]);
    fileBlobsRef.current.clear();
    setFileNotice(null);
    setPasteNotice(null);
    setTrigger(null);
    // Attachments belong to one session's draft; they never follow a switch.
  }, [draftBinding?.productSessionId, draftBinding?.workspaceId]);

  // A smart stop can restore the send as it was, chips included (design F6).
  // The store carries the chips because the sent message has them folded into
  // its text, and un-folding that would be guesswork. A text-only restore means
  // the chips that were in the composer no longer match it, so they go away.
  const lastRestore = draft.lastRestore;
  useEffect(() => {
    if (lastRestore === null) {
      return;
    }
    const chips = lastRestore.source === "snapshot" ? lastRestore.chips : [];
    setAttachments(chips.map((chip) => ({ id: createAttachmentId(), text: chip.text })));
    setPasteNotice(
      chips.length > 0 ? t("chat.smartStopRestoredPastes", { n: chips.length }) : null,
    );
    // R8: the snapshot carries references, not bytes, so a restored row is
    // shown immediately and then confirmed against the server. A reference the
    // server no longer resolves turns into a visible re-attach row instead of
    // an id that only fails once the message is already on its way.
    const restored =
      lastRestore.source === "snapshot"
        ? rowsFromSendSnapshot(lastRestore.attachments, createAttachmentId)
        : [];
    setFiles(restored);
    setFileNotice(null);
    fileBlobsRef.current.clear();
    if (onProbeAttachment) {
      const probe = onProbeAttachment;
      for (const row of restored) {
        const attachmentId = row.attachmentId;
        if (!attachmentId) {
          continue;
        }
        void probe(attachmentId).catch((error: unknown) => {
          const failure = probeFailureFromError(error);
          setFiles((current) =>
            current.map((candidate) =>
              candidate.id === row.id && candidate.attachmentId === attachmentId
                ? { ...candidate, status: "failed", failure, retrying: false, restored: false }
                : candidate,
            ),
          );
        });
      }
    }
    // The store hands over a fresh object per restore; the copy function is
    // stable and must not re-fire this.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lastRestore]);

  const [pendingSend, setPendingSend] = useState<string | null>(null);
  /**
   * The draft a deferred send was armed with.
   *
   * The pause is short, but not instantaneous: a reader can keep editing while
   * it lasts. A send only goes out if the draft is the one that was submitted —
   * anything else and the press is spent on the version the reader has since
   * changed, which is nothing anyone asked to send.
   */
  const pendingSendRef = useRef<{
    text: string;
    chips: string[];
    files: string[];
  } | null>(null);

  const canSubmit =
    Boolean(message.trim()) &&
    !submitting &&
    !disabled &&
    !sendPaused &&
    // A row that is still uploading, still retrying, or already failed is not
    // something a send may quietly drop: the attachment stays visible until the
    // reader removes it or the upload lands.
    !hasUnfinishedFiles(files);

  useEffect(() => {
    const armed = pendingSendRef.current;
    if (!armed) {
      return;
    }
    // A structural lock replaces the pause: the draft is still the reader's, so
    // it stays armed for the next window rather than being sent into a state the
    // shell cannot bind a send to.
    if (disabled) {
      return;
    }
    const changed =
      armed.text !== message ||
      armed.chips.length !== attachments.length ||
      armed.chips.some((chip, index) => chip !== attachments[index]?.text) ||
      armed.files.length !== files.length ||
      armed.files.some((id, index) => id !== files[index]?.id);
    if (changed) {
      pendingSendRef.current = null;
      setPendingSend(null);
      return;
    }
    if (sendPaused || submitting || !canSubmit) {
      return;
    }
    pendingSendRef.current = null;
    setPendingSend(null);
    void submitDraft();
    // `submitDraft` reads the current draft, attachments, and rows; every value
    // it reads is in this list.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    attachments,
    canSubmit,
    disabled,
    files,
    message,
    sendPaused,
    submitting,
  ]);

  function failureText(failure: ComposerFileFailure, name: string): string {
    return t(failureCopyKey(failure), {
      name,
      limit: formatAttachmentBytes(attachmentByteCap(name)),
      n: MAX_FILE_ATTACHMENTS,
    });
  }

  /**
   * Upload one row, then keep it in step with the result. A failure the server
   * may answer differently next time gets exactly one retry, and that retry is
   * a new upload: an attempt that failed never left an id behind, so a retry
   * cannot reuse one.
   */
  async function uploadFile(rowId: string, file: File): Promise<void> {
    const upload = onUploadAttachment;
    if (!upload) {
      return;
    }
    const attempt = async (isRetry: boolean): Promise<void> => {
      setFiles((current) =>
        current.map((row) =>
          row.id === rowId
            ? { ...row, status: "uploading", failure: undefined, retrying: isRetry }
            : row,
        ),
      );
      try {
        const response = await upload(file, file.name);
        fileBlobsRef.current.delete(rowId);
        setFiles((current) =>
          current.map((row) => (row.id === rowId ? rowFromUpload(rowId, file.name, response) : row)),
        );
      } catch (error) {
        const failure = uploadFailureFromError(error);
        if (!isRetry && isRetryableFailure(failure)) {
          await attempt(true);
          return;
        }
        setFiles((current) =>
          current.map((row) =>
            row.id === rowId ? { ...row, status: "failed", failure, retrying: false } : row,
          ),
        );
      }
    };
    await attempt(false);
  }

  function addFiles(picked: readonly File[]): void {
    if (!onUploadAttachment || picked.length === 0) {
      return;
    }
    const decisions = decideFileSelection(
      picked.map((file) => ({ name: file.name, size: file.size })),
      files.length,
    );
    const accepted: { row: ComposerFileRow; file: File }[] = [];
    let refusal: { failure: ComposerFileFailure; name: string } | null = null;
    for (let index = 0; index < picked.length; index += 1) {
      const file = picked[index];
      const decision = decisions[index];
      if (decision.kind === "accept") {
        accepted.push({
          row: {
            id: createAttachmentId(),
            name: file.name,
            size: file.size,
            status: "uploading",
            warnings: [],
            retrying: false,
            restored: false,
          },
          file,
        });
        continue;
      }
      // One notice per selection: the first refusal is the one worth naming,
      // and the row-level failures below carry everything else.
      if (refusal === null) {
        refusal = { failure: decision.failure, name: file.name };
      }
    }
    if (accepted.length > 0) {
      for (const entry of accepted) {
        fileBlobsRef.current.set(entry.row.id, entry.file);
      }
      setFiles((current) => [...current, ...accepted.map((entry) => entry.row)]);
      setFileNotice(null);
    }
    if (refusal !== null) {
      setFileNotice(failureText(refusal.failure, refusal.name));
    }
    for (const entry of accepted) {
      void uploadFile(entry.row.id, entry.file);
    }
  }

  function removeFile(rowId: string): void {
    fileBlobsRef.current.delete(rowId);
    setFiles((current) => current.filter((row) => row.id !== rowId));
    setFileNotice(null);
  }

  function retryFile(row: ComposerFileRow): void {
    const file = fileBlobsRef.current.get(row.id);
    if (!file) {
      return;
    }
    void uploadFile(row.id, file);
  }

  async function submitDraft() {
    if (!canSubmit) {
      if (disabled || submitting) {
        return;
      }
      if (sendPaused && message.trim() && !hasUnfinishedFiles(files)) {
        // The one press a stop can swallow: the transcript is being re-read for
        // the session on screen, so the send waits for it instead of vanishing.
        // An unfinished attachment still refuses, exactly as it does when the
        // composer is idle, because the bytes are not the server's yet.
        pendingSendRef.current = {
          text: message,
          chips: attachments.map((attachment) => attachment.text),
          files: files.map((row) => row.id),
        };
        setPendingSend(message);
      }
      return;
    }
    // Capture the chips and the attachment references before the send clears
    // them: an accepted send keeps both as this draft's restore snapshot.
    const sentChips = attachments.map((attachment) => ({ text: attachment.text }));
    const sentFiles = sendReferencesFromRows(files);
    const sentAttachments = requestAttachmentsForSend(files);
    await draft.submit(
      async (sent) => {
        const accepted = await onSend(
          composeMessageWithAttachments(sent, attachments),
          sentAttachments,
        );
        if (accepted) {
          setAttachments([]);
          setFiles([]);
          fileBlobsRef.current.clear();
          setPasteNotice(null);
          setFileNotice(null);
        }
        return accepted;
      },
      sentChips,
      sentFiles,
    );
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
    <form
      className="chat-composer"
      onSubmit={handleSubmit}
      aria-label={t("chat.placeholder")}
      data-drop-active={dropActive ? "true" : undefined}
      onDragOver={(event) => {
        if (!onUploadAttachment || !event.dataTransfer.types.includes("Files")) {
          return;
        }
        // Only a drag that carries files is claimed; a text drag keeps its
        // native target behavior.
        event.preventDefault();
        setDropActive(true);
      }}
      onDragLeave={(event) => {
        if (event.currentTarget.contains(event.relatedTarget as Node | null)) {
          return;
        }
        setDropActive(false);
      }}
      onDrop={(event) => {
        if (!onUploadAttachment) {
          return;
        }
        const picked = Array.from(event.dataTransfer.files);
        setDropActive(false);
        if (picked.length === 0) {
          return;
        }
        event.preventDefault();
        addFiles(picked);
      }}
    >
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
        {pendingSend !== null ? (
          <span role="status">{t("chat.sendQueuedRestoring")}</span>
        ) : null}
        {queuedEditNotice ? <span>{queuedEditNotice}</span> : null}
      </div>
      {dropActive ? (
        <p className="chat-composer__drop-hint" role="status">
          {t("chat.attachmentDropHint")}
        </p>
      ) : null}
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
      {files.length > 0 ? (
        <div className="chat-composer__files">
          {files.map((row) => (
            <div
              className="file-chip"
              key={row.id}
              data-status={row.status}
              data-restored={row.restored ? "true" : undefined}
            >
              <span className="file-chip__name" title={row.name}>
                {row.name}
              </span>
              <span className="file-chip__meta">
                {row.status === "uploading"
                  ? row.retrying
                    ? t("chat.attachmentRetrying")
                    : t("chat.attachmentUploading")
                  : row.status === "uploaded"
                    ? row.restored
                      ? t("chat.attachmentRestored")
                      : t("chat.attachmentUploaded")
                    : null}
                {row.size > 0 ? ` · ${formatAttachmentBytes(row.size)}` : ""}
              </span>
              {row.status === "uploading" ? (
                // Indeterminate on purpose: `fetch` reports no bytes-uploaded
                // progress, and the server has not answered yet either, so a
                // percentage here would be invented.
                <span
                  className="file-chip__progress"
                  role="progressbar"
                  aria-label={t("chat.attachmentUploading")}
                />
              ) : null}
              {row.status === "failed" && row.failure ? (
                <span className="file-chip__error" role="alert">
                  {failureText(row.failure, row.name)}
                </span>
              ) : null}
              {row.warnings.map((code) => (
                <span className="file-chip__warning" key={code} data-code={code}>
                  {t(warningCopyKey(code))}
                </span>
              ))}
              {row.status === "failed" &&
              row.failure &&
              isRetryableFailure(row.failure) &&
              fileBlobsRef.current.has(row.id) ? (
                // A manual retry is offered on the same terms as the automatic
                // one: only a failure the server may answer differently next
                // time, and only while the bytes are still here to send.
                <button type="button" className="secondary" onClick={() => retryFile(row)}>
                  {t("chat.retry")}
                </button>
              ) : null}
              <button
                type="button"
                className="icon-button"
                aria-label={t("chat.attachmentRemove", { name: row.name })}
                onClick={() => removeFile(row.id)}
              >
                ×
              </button>
            </div>
          ))}
        </div>
      ) : null}
      {fileNotice ? (
        <p className="chat-composer__paste-notice" role="status">
          {fileNotice}
        </p>
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
        {onUploadAttachment ? (
          <>
            <input
              ref={fileInputRef}
              type="file"
              multiple
              accept={ATTACHMENT_ACCEPT}
              className="chat-composer__file-input"
              onChange={(event) => {
                // The value is cleared first so picking the same file twice in
                // a row still fires a change event.
                const picked = Array.from(event.target.files ?? []);
                event.target.value = "";
                addFiles(picked);
              }}
            />
            <button
              type="button"
              className="icon-button chat-composer__attach"
              aria-label={t("chat.attachFile")}
              title={t("chat.attachFileHint")}
              disabled={disabled || submitting}
              onClick={() => fileInputRef.current?.click()}
            >
              <PlusIcon />
            </button>
          </>
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
