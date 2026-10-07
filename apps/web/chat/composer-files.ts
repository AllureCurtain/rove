/**
 * File attachments for the composer (R8 PR-5 / frontend F13).
 *
 * Pure state machine, no React: the composer owns ids, focus, and rendering;
 * this module owns the caps, the local pre-checks, the server-code to copy-key
 * mapping, and the one function that turns rows into a send request.
 *
 * The caps mirror `apps/api/src/product/contracts.rs` and
 * `runtime/src/conversation.rs` so a doomed upload is refused in the browser
 * instead of after 20 MiB of bytes, and the allow-list is the server's own
 * `ALLOWED_ATTACHMENT_EXTENSIONS`: this module can narrow a selection, never
 * widen it. The server remains the authority — every local decision here is
 * re-made there, and its refusal is what the UI shows.
 */

import type {
  ProductAttachmentUpload,
  ProductMessageAttachmentRef,
  ProductMessageAttachmentRequest,
} from "../product/product-api-types";
import { ProductApiSchemaError } from "../product/product-api-types";
import type { ComposerSendReference } from "../state/composer-draft-store";
import { ProductApiError } from "../product/product-client";

/** `MAX_MESSAGE_ATTACHMENTS`, and the staged/referenced per-session cap. */
export const MAX_FILE_ATTACHMENTS = 32;
/** Raster bytes cap (`MAX_PRODUCT_ATTACHMENT_RASTER_BYTES`). */
export const MAX_FILE_RASTER_BYTES = 16 * 1024 * 1024;
/** Document bytes cap (`MAX_PRODUCT_ATTACHMENT_DOCUMENT_BYTES`). */
export const MAX_FILE_DOCUMENT_BYTES = 20 * 1024 * 1024;

/**
 * The server's `ALLOWED_ATTACHMENT_EXTENSIONS`, in its order. Kept as
 * extensions rather than MIME types because the extension is what the browser
 * gives us before a byte is read, and it is also what the server sniffs against.
 */
export const ALLOWED_ATTACHMENT_EXTENSIONS = [
  "png",
  "jpg",
  "jpeg",
  "webp",
  "gif",
  "pdf",
  "txt",
  "md",
] as const;

const RASTER_EXTENSIONS = new Set(["png", "jpg", "jpeg", "webp", "gif"]);

/** The file-input `accept` value, derived so it cannot drift from the list. */
export const ATTACHMENT_ACCEPT = ALLOWED_ATTACHMENT_EXTENSIONS.join(",");

/** The verified media type the download route serves inline. */
const RASTER_CONTENT_TYPES = new Set([
  "image/png",
  "image/jpeg",
  "image/gif",
  "image/webp",
]);

/**
 * The three server warning codes (`apps/api/src/product/attachments.rs`), plus
 * an `other` bucket: the response is `warnings: string[]` and a client that
 * meets a code it does not know must still say something.
 */
export const ATTACHMENT_WARNING_CODES = [
  "secret_shaped_name",
  "possible_secret_content",
  "content_type_claim_mismatch",
] as const;
export type AttachmentWarningCode = (typeof ATTACHMENT_WARNING_CODES)[number];

/** Every way a row can end up not sendable, in the UI's own vocabulary. */
export type ComposerFileFailure =
  | "too-large"
  | "unsupported-type"
  | "too-many"
  | "quota"
  | "busy"
  | "timeout"
  | "unavailable"
  | "invalid"
  | "expired"
  | "missing"
  | "too-large-for-model"
  | "network"
  | "unexpected";

export type ComposerFileStatus = "uploading" | "uploaded" | "failed";

/** One file row in the composer. Ids are composer-local and never sent. */
export interface ComposerFileRow {
  /** Composer-local row identity; a retry keeps it and mints a new upload. */
  id: string;
  name: string;
  size: number;
  status: ComposerFileStatus;
  /** Present once the server confirmed the upload. */
  attachmentId?: string;
  contentType?: string;
  sha256?: string;
  warnings: string[];
  failure?: ComposerFileFailure;
  /** The row is waiting on a retry this composer scheduled. */
  retrying: boolean;
  /** The row came back from a draft restore and the server confirmed it. */
  restored: boolean;
}

export interface ComposerFileCandidate {
  name: string;
  size: number;
}

export type FileSelectionDecision =
  | { kind: "accept" }
  | { kind: "reject"; failure: "too-many" | "too-large" | "unsupported-type" };

/** The lowercase extension without the dot; empty when the name has none. */
export function attachmentExtension(name: string): string {
  const trimmed = name.trim();
  const dot = trimmed.lastIndexOf(".");
  if (dot <= 0 || dot === trimmed.length - 1) {
    return "";
  }
  return trimmed.slice(dot + 1).toLowerCase();
}

export function isAllowedAttachmentName(name: string): boolean {
  const extension = attachmentExtension(name);
  return (ALLOWED_ATTACHMENT_EXTENSIONS as readonly string[]).includes(extension);
}

export function isRasterAttachmentExtension(extension: string): boolean {
  return RASTER_EXTENSIONS.has(extension.toLowerCase());
}

/** The byte cap that applies to a name, before any byte is read. */
export function attachmentByteCap(name: string): number {
  return isRasterAttachmentExtension(attachmentExtension(name))
    ? MAX_FILE_RASTER_BYTES
    : MAX_FILE_DOCUMENT_BYTES;
}

/**
 * Decide what to do with a picked or dropped selection. `count` is how many
 * rows the composer already holds, so the cap is enforced across selections
 * rather than per selection.
 */
export function decideFileSelection(
  candidates: readonly ComposerFileCandidate[],
  count: number,
): FileSelectionDecision[] {
  let accepted = count;
  return candidates.map((candidate) => {
    if (!isAllowedAttachmentName(candidate.name)) {
      return { kind: "reject", failure: "unsupported-type" } as const;
    }
    const cap = attachmentByteCap(candidate.name);
    if (candidate.size > cap) {
      return { kind: "reject", failure: "too-large" } as const;
    }
    if (accepted >= MAX_FILE_ATTACHMENTS) {
      return { kind: "reject", failure: "too-many" } as const;
    }
    accepted += 1;
    return { kind: "accept" } as const;
  });
}

/** Rows that still block a send: an unconfirmed upload or a failed row. */
export function hasUnfinishedFiles(rows: readonly ComposerFileRow[]): boolean {
  return rows.some((row) => row.status !== "uploaded");
}

/**
 * The request list for a send: confirmed rows, in the order the user added
 * them. A row the server never confirmed is never sent, so a message cannot
 * reference an id that does not exist.
 */
export function requestAttachmentsForSend(
  rows: readonly ComposerFileRow[],
): ProductMessageAttachmentRequest[] {
  const request: ProductMessageAttachmentRequest[] = [];
  for (const row of rows) {
    if (row.status !== "uploaded" || !row.attachmentId) {
      continue;
    }
    request.push({ attachment_id: row.attachmentId, name: row.name });
  }
  return request;
}

/** Build the confirmed row for one upload response. */
export function rowFromUpload(
  id: string,
  name: string,
  upload: ProductAttachmentUpload,
): ComposerFileRow {
  return {
    id,
    name: upload.name ?? name,
    size: upload.size,
    status: "uploaded",
    attachmentId: upload.attachment_id,
    contentType: upload.content_type,
    sha256: upload.sha256,
    warnings: [...upload.warnings],
    retrying: false,
    restored: false,
  };
}

/**
 * Rows restored from a send snapshot. The snapshot carries references, not
 * bytes, so a restored row is only re-validated by the server: `probeAttachment`
 * confirms it, and these rows are what the composer shows until it answers.
 */
export function rowsFromSendSnapshot(
  references: readonly ComposerSendReference[],
  nextId: () => string,
): ComposerFileRow[] {
  return references.map((reference) => ({
    id: nextId(),
    name: reference.name ?? "attachment",
    size: reference.size ?? 0,
    status: "uploaded" as const,
    attachmentId: reference.attachment_id,
    contentType: reference.content_type,
    sha256: reference.sha256,
    warnings: [],
    retrying: false,
    restored: true,
  }));
}

/** Snapshot the references of the rows a send actually carried. */
export function sendReferencesFromRows(
  rows: readonly ComposerFileRow[],
): ComposerSendReference[] {
  return rows
    .filter((row) => row.status === "uploaded" && row.attachmentId)
    .map((row) => ({
      attachment_id: row.attachmentId as string,
      name: row.name,
      content_type: row.contentType,
      size: row.size,
      sha256: row.sha256,
    }));
}

/**
 * Map a thrown probe error onto the row vocabulary. The probe and the upload
 * share codes but not meanings: `product_attachment_unavailable` means the
 * storage refused an upload, while for a reference it means the bytes the row
 * still points at are gone.
 */
export function probeFailureFromError(error: unknown): ComposerFileFailure {
  if (error instanceof ProductApiError) {
    switch (error.code) {
      case "product_attachment_conflict":
        return "expired";
      case "product_attachment_unavailable":
        return "missing";
      case "product_attachment_not_found":
        return "invalid";
      case "product_attachment_busy":
        return "busy";
      default:
        return "unexpected";
    }
  }
  return "network";
}

/** The copy key for one server warning code. */
export function warningCopyKey(code: string): string {
  switch (code) {
    case "secret_shaped_name":
      return "chat.attachmentWarningSecretName";
    case "possible_secret_content":
      return "chat.attachmentWarningSecretContent";
    case "content_type_claim_mismatch":
      return "chat.attachmentWarningTypeMismatch";
    default:
      return "chat.attachmentWarningOther";
  }
}

/** The copy key for one failure. */
export function failureCopyKey(failure: ComposerFileFailure): string {
  switch (failure) {
    case "too-large":
      return "chat.attachmentFailureTooLarge";
    case "too-large-for-model":
      return "chat.attachmentFailureTooLargeForModel";
    case "unsupported-type":
      return "chat.attachmentFailureUnsupported";
    case "too-many":
      return "chat.attachmentFailureTooMany";
    case "quota":
      return "chat.attachmentFailureQuota";
    case "busy":
      return "chat.attachmentFailureBusy";
    case "timeout":
      return "chat.attachmentFailureTimeout";
    case "unavailable":
      return "chat.attachmentFailureUnavailable";
    case "invalid":
      return "chat.attachmentFailureInvalid";
    case "expired":
      return "chat.attachmentFailureExpired";
    case "missing":
      return "chat.attachmentFailureMissing";
    case "network":
      return "chat.attachmentFailureNetwork";
    default:
      return "chat.attachmentFailureUnexpected";
  }
}

/** A retry is only owed to a failure the server may answer differently next time. */
export function isRetryableFailure(failure: ComposerFileFailure): boolean {
  return failure === "busy" || failure === "timeout" || failure === "network";
}

/** Map a thrown upload error onto the row vocabulary. */
export function uploadFailureFromError(error: unknown): ComposerFileFailure {
  if (error instanceof ProductApiError) {
    switch (error.code) {
      case "product_attachment_too_large":
        return "too-large";
      case "product_attachment_invalid_input":
        return "unsupported-type";
      case "product_attachment_quota":
        return "quota";
      case "product_attachment_busy":
        return "busy";
      case "product_attachment_timeout":
        return "timeout";
      case "product_attachment_unavailable":
        return "unavailable";
      case "product_attachment_conflict":
        return "expired";
      case "product_attachment_not_found":
        return "invalid";
      default:
        return "unexpected";
    }
  }
  if (error instanceof ProductApiSchemaError) {
    return "unexpected";
  }
  // A refused connection, a dropped request, or an aborted fetch. `fetch`
  // rejects with a TypeError for all three; none of them proves the server
  // saw the bytes, so the honest word is "the upload did not arrive".
  return "network";
}

/** The availability copy key for a durable reference. */
export function availabilityCopyKey(availability: string): string | null {
  switch (availability) {
    case "missing":
      return "chat.attachmentAvailabilityMissing";
    case "corrupt":
      return "chat.attachmentAvailabilityCorrupt";
    case "expired":
      return "chat.attachmentAvailabilityExpired";
    default:
      return null;
  }
}

/** Whether a verified media type is served inline, so `<img src>` works. */
export function isRasterContentType(contentType: string): boolean {
  return RASTER_CONTENT_TYPES.has(contentType.split(";")[0].trim().toLowerCase());
}

/** Byte size the way the rest of the shell writes it. */
export function formatAttachmentBytes(value: number): string {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MiB`;
}

/** A short, stable identity prefix for a reference row. */
export function shortAttachmentHash(sha256: string | undefined): string {
  if (!sha256) return "";
  return sha256.slice(0, 12);
}

/** The same reference set regardless of order, for the send request key. */
export function attachmentRequestKey(
  attachments: readonly ProductMessageAttachmentRequest[],
): string {
  return attachments
    .map((attachment) => attachment.attachment_id)
    .sort()
    .join(",");
}

/** Durable references the transcript can render for one run. */
export function attachmentsByRunId(
  messages: readonly {
    run_id?: string | null;
    successor_run_id?: string | null;
    attachments?: readonly ProductMessageAttachmentRef[];
  }[],
): Map<string, ProductMessageAttachmentRef[]> {
  const byRun = new Map<string, ProductMessageAttachmentRef[]>();
  for (const message of messages) {
    const attachments = message.attachments ?? [];
    if (attachments.length === 0) {
      continue;
    }
    for (const runId of [message.run_id, message.successor_run_id]) {
      if (!runId) continue;
      const existing = byRun.get(runId);
      if (existing) {
        existing.push(...attachments);
      } else {
        byRun.set(runId, [...attachments]);
      }
    }
  }
  return byRun;
}
