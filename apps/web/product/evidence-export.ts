import type { ProductSessionEvidenceDownload } from "./product-client";

export function downloadEvidenceFile(download: ProductSessionEvidenceDownload): void {
  if (typeof document === "undefined" || typeof URL === "undefined") {
    throw new Error("Evidence downloads require a browser environment.");
  }
  const objectUrl = URL.createObjectURL(download.content);
  const anchor = document.createElement("a");
  anchor.href = objectUrl;
  anchor.download = download.filename;
  anchor.rel = "noopener";
  anchor.hidden = true;
  document.body.append(anchor);
  try {
    anchor.click();
  } finally {
    anchor.remove();
    URL.revokeObjectURL(objectUrl);
  }
}
