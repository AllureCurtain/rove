import { describe, expect, it } from "vitest";

import { ProductApiSchemaError } from "../product/product-api-types";
import { ProductApiError } from "../product/product-client";
import {
  ALLOWED_ATTACHMENT_EXTENSIONS,
  ATTACHMENT_ACCEPT,
  MAX_FILE_ATTACHMENTS,
  MAX_FILE_DOCUMENT_BYTES,
  MAX_FILE_RASTER_BYTES,
  attachmentByteCap,
  attachmentExtension,
  attachmentRequestKey,
  attachmentsByRunId,
  availabilityCopyKey,
  decideFileSelection,
  failureCopyKey,
  formatAttachmentBytes,
  hasUnfinishedFiles,
  isAllowedAttachmentName,
  isRasterContentType,
  isRetryableFailure,
  probeFailureFromError,
  requestAttachmentsForSend,
  rowFromUpload,
  rowsFromSendSnapshot,
  sendReferencesFromRows,
  shortAttachmentHash,
  uploadFailureFromError,
  warningCopyKey,
  type ComposerFileFailure,
  type ComposerFileRow,
} from "./composer-files";

const upload = {
  attachment_id: "01J00000000000000000000031",
  product_session_id: "01J00000000000000000000002",
  content_type: "image/png",
  size: 2048,
  sha256: "a".repeat(64),
  name: "shot.png",
  status: "staged" as const,
  warnings: ["secret_shaped_name"],
};

function row(overrides: Partial<ComposerFileRow> = {}): ComposerFileRow {
  return {
    id: "row-1",
    name: "shot.png",
    size: 2048,
    status: "uploaded",
    attachmentId: "att-1",
    warnings: [],
    retrying: false,
    restored: false,
    ...overrides,
  };
}

describe("composer file attachments", () => {
  it("accepts exactly the server's extension allow-list", () => {
    for (const extension of ALLOWED_ATTACHMENT_EXTENSIONS) {
      expect(isAllowedAttachmentName(`file.${extension}`)).toBe(true);
      expect(isAllowedAttachmentName(`FILE.${extension.toUpperCase()}`)).toBe(true);
    }
    for (const name of ["run.exe", "archive.zip", "noextension", ".png", "notes.md.exe"]) {
      expect(isAllowedAttachmentName(name)).toBe(false);
    }
    expect(ATTACHMENT_ACCEPT).toBe(ALLOWED_ATTACHMENT_EXTENSIONS.join(","));
  });

  it("reads the extension without trusting dots in directories", () => {
    expect(attachmentExtension("a/b.c/d.txt")).toBe("txt");
    expect(attachmentExtension("  spaced.png  ")).toBe("png");
    expect(attachmentExtension("trailing.")).toBe("");
  });

  it("applies the raster cap to images and the document cap to everything else", () => {
    expect(attachmentByteCap("a.png")).toBe(MAX_FILE_RASTER_BYTES);
    expect(attachmentByteCap("a.WEBP")).toBe(MAX_FILE_RASTER_BYTES);
    expect(attachmentByteCap("a.pdf")).toBe(MAX_FILE_DOCUMENT_BYTES);
    expect(MAX_FILE_RASTER_BYTES).toBeLessThan(MAX_FILE_DOCUMENT_BYTES);
  });

  it("refuses an oversized image but accepts an equally sized document", () => {
    const size = MAX_FILE_RASTER_BYTES + 1;
    expect(decideFileSelection([{ name: "big.png", size }], 0)).toEqual([
      { kind: "reject", failure: "too-large" },
    ]);
    expect(decideFileSelection([{ name: "big.pdf", size }], 0)).toEqual([
      { kind: "accept" },
    ]);
  });

  it("accepts a file exactly at the cap and refuses one byte past it", () => {
    expect(
      decideFileSelection([{ name: "edge.pdf", size: MAX_FILE_DOCUMENT_BYTES }], 0),
    ).toEqual([{ kind: "accept" }]);
    expect(
      decideFileSelection([{ name: "edge.pdf", size: MAX_FILE_DOCUMENT_BYTES + 1 }], 0),
    ).toEqual([{ kind: "reject", failure: "too-large" }]);
  });

  it("counts the message cap across selections, not within one", () => {
    const decisions = decideFileSelection(
      [
        { name: "a.txt", size: 1 },
        { name: "b.txt", size: 1 },
        { name: "c.txt", size: 1 },
      ],
      MAX_FILE_ATTACHMENTS - 1,
    );
    expect(decisions).toEqual([
      { kind: "accept" },
      { kind: "reject", failure: "too-many" },
      { kind: "reject", failure: "too-many" },
    ]);
  });

  it("refuses an unsupported type before it measures bytes", () => {
    expect(
      decideFileSelection([{ name: "huge.exe", size: 10 * MAX_FILE_DOCUMENT_BYTES }], 0),
    ).toEqual([{ kind: "reject", failure: "unsupported-type" }]);
  });

  it("sends only confirmed rows, in the order they were added", () => {
    const rows = [
      row({ id: "a", attachmentId: "att-a" }),
      row({ id: "b", status: "uploading", attachmentId: undefined }),
      row({ id: "c", attachmentId: "att-c", name: "notes.md" }),
      row({ id: "d", status: "failed", failure: "busy", attachmentId: undefined }),
    ];
    expect(requestAttachmentsForSend(rows)).toEqual([
      { attachment_id: "att-a", name: "shot.png" },
      { attachment_id: "att-c", name: "notes.md" },
    ]);
    expect(sendReferencesFromRows(rows).map((ref) => ref.attachment_id)).toEqual([
      "att-a",
      "att-c",
    ]);
  });

  it("treats every unconfirmed row as unfinished and blockers", () => {
    expect(hasUnfinishedFiles([row()])).toBe(false);
    expect(hasUnfinishedFiles([row({ status: "uploading" })])).toBe(true);
    expect(hasUnfinishedFiles([row({ status: "failed", failure: "busy" })])).toBe(true);
  });

  it("restores a snapshot as references the server still has to confirm", () => {
    const rows = rowsFromSendSnapshot(
      [{ attachment_id: "att-1", name: "shot.png", content_type: "image/png", size: 12 }],
      () => "row-9",
    );
    expect(rows).toEqual([
      {
        id: "row-9",
        name: "shot.png",
        size: 12,
        status: "uploaded",
        attachmentId: "att-1",
        contentType: "image/png",
        sha256: undefined,
        warnings: [],
        retrying: false,
        restored: true,
      },
    ]);
    // A restored row is sendable, so a stop that put it back can send again.
    expect(requestAttachmentsForSend(rows)).toEqual([
      { attachment_id: "att-1", name: "shot.png" },
    ]);
  });

  it("takes the server's verified values from an upload response", () => {
    expect(rowFromUpload("row-2", "claim.txt", upload)).toEqual({
      id: "row-2",
      name: "shot.png",
      size: 2048,
      status: "uploaded",
      attachmentId: upload.attachment_id,
      contentType: "image/png",
      sha256: "a".repeat(64),
      warnings: ["secret_shaped_name"],
      retrying: false,
      restored: false,
    });
  });

  it("keeps the request key independent of the order the references arrive", () => {
    expect(
      attachmentRequestKey([
        { attachment_id: "b" },
        { attachment_id: "a" },
      ]),
    ).toBe("a,b");
    expect(attachmentRequestKey([])).toBe("");
  });

  it("maps every warning code onto its own copy key", () => {
    expect(warningCopyKey("secret_shaped_name")).toBe("chat.attachmentWarningSecretName");
    expect(warningCopyKey("possible_secret_content")).toBe(
      "chat.attachmentWarningSecretContent",
    );
    expect(warningCopyKey("content_type_claim_mismatch")).toBe(
      "chat.attachmentWarningTypeMismatch",
    );
    expect(warningCopyKey("something_new")).toBe("chat.attachmentWarningOther");
  });

  it("maps every failure onto its own copy key", () => {
    const failures: ComposerFileFailure[] = [
      "too-large",
      "too-large-for-model",
      "unsupported-type",
      "too-many",
      "quota",
      "busy",
      "timeout",
      "unavailable",
      "invalid",
      "expired",
      "missing",
      "network",
      "unexpected",
    ];
    const keys = failures.map((failure) => failureCopyKey(failure));
    expect(new Set(keys).size).toBe(failures.length);
    for (const key of keys) {
      expect(key.startsWith("chat.attachmentFailure")).toBe(true);
    }
  });

  it("retries only a failure the server may answer differently next time", () => {
    expect(isRetryableFailure("busy")).toBe(true);
    expect(isRetryableFailure("timeout")).toBe(true);
    expect(isRetryableFailure("network")).toBe(true);
    expect(isRetryableFailure("too-large")).toBe(false);
    expect(isRetryableFailure("quota")).toBe(false);
    expect(isRetryableFailure("invalid")).toBe(false);
  });

  it("maps an upload refusal onto the row vocabulary", () => {
    const cases: [string, ComposerFileFailure][] = [
      ["product_attachment_too_large", "too-large"],
      ["product_attachment_invalid_input", "unsupported-type"],
      ["product_attachment_quota", "quota"],
      ["product_attachment_busy", "busy"],
      ["product_attachment_timeout", "timeout"],
      ["product_attachment_unavailable", "unavailable"],
      ["product_attachment_conflict", "expired"],
      ["product_attachment_not_found", "invalid"],
      ["something_else", "unexpected"],
    ];
    for (const [code, failure] of cases) {
      expect(uploadFailureFromError(new ProductApiError(413, code, "no"))).toBe(failure);
    }
    expect(uploadFailureFromError(new ProductApiSchemaError("bad body"))).toBe("unexpected");
    // A refused connection proves nothing about whether the server saw bytes.
    expect(uploadFailureFromError(new TypeError("Failed to fetch"))).toBe("network");
  });

  it("reads a probe refusal as the state of the reference, not of storage", () => {
    expect(
      probeFailureFromError(new ProductApiError(409, "product_attachment_conflict", "gone")),
    ).toBe("expired");
    expect(
      probeFailureFromError(
        new ProductApiError(410, "product_attachment_unavailable", "gone"),
      ),
    ).toBe("missing");
    expect(
      probeFailureFromError(
        new ProductApiError(404, "product_attachment_not_found", "gone"),
      ),
    ).toBe("invalid");
    expect(probeFailureFromError(new TypeError("Failed to fetch"))).toBe("network");
  });

  it("only names an availability the server can report", () => {
    expect(availabilityCopyKey("missing")).toBe("chat.attachmentAvailabilityMissing");
    expect(availabilityCopyKey("corrupt")).toBe("chat.attachmentAvailabilityCorrupt");
    expect(availabilityCopyKey("expired")).toBe("chat.attachmentAvailabilityExpired");
    expect(availabilityCopyKey("available")).toBeNull();
  });

  it("previews only a verified raster type", () => {
    expect(isRasterContentType("image/png")).toBe(true);
    expect(isRasterContentType("IMAGE/JPEG; charset=binary")).toBe(true);
    expect(isRasterContentType("application/pdf")).toBe(false);
    expect(isRasterContentType("text/plain")).toBe(false);
  });

  it("writes sizes the way the rest of the shell does", () => {
    expect(formatAttachmentBytes(0)).toBe("0 B");
    expect(formatAttachmentBytes(1023)).toBe("1023 B");
    expect(formatAttachmentBytes(1024)).toBe("1.0 KiB");
    expect(formatAttachmentBytes(1024 * 1024)).toBe("1.0 MiB");
    expect(shortAttachmentHash("abcdef0123456789")).toBe("abcdef012345");
    expect(shortAttachmentHash(undefined)).toBe("");
  });

  it("groups durable references by the run that carried them", () => {
    const reference = {
      attachment_id: "att-1",
      content_type: "image/png",
      size: 1,
      sha256: "b".repeat(64),
      availability: "available" as const,
    };
    const byRun = attachmentsByRunId([
      { run_id: "run-1", attachments: [reference] },
      { run_id: "run-2", successor_run_id: "run-3", attachments: [reference] },
      // A message with no attachments contributes nothing, and a run that
      // never carried one is simply absent.
      { run_id: "run-4" },
      { attachments: [reference] },
    ]);
    expect(byRun.get("run-1")?.map((ref) => ref.attachment_id)).toEqual(["att-1"]);
    // A successor message's references belong to the run it was claimed by.
    expect(byRun.get("run-2")?.map((ref) => ref.attachment_id)).toEqual(["att-1"]);
    expect(byRun.get("run-3")?.map((ref) => ref.attachment_id)).toEqual(["att-1"]);
    expect(byRun.get("run-4")).toBeUndefined();
    expect(byRun.size).toBe(3);
  });
});
