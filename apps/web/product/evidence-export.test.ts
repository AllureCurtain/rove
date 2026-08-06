import { describe, expect, it, vi } from "vitest";

import { downloadEvidenceFile } from "./evidence-export";
import {
  createProductApiClient,
  type ProductExportFormat,
} from "./product-client";
import { ProductApiSchemaError } from "./product-api-types";

describe("product evidence export", () => {
  it.each([
    ["json", "application/json", "json"],
    ["html", "text/html", "html"],
    ["markdown", "text/markdown", "md"],
  ] as const)("downloads the %s response without parsing away evidence", async (format, mediaType, extension) => {
    const fetchMock = vi.fn(async () => new Response(`evidence-${format}`, {
      status: 200,
      headers: {
        "content-type": `${mediaType}; charset=utf-8`,
        "content-disposition": `attachment; filename="rove-session-safe-evidence.${extension}"`,
      },
    }));
    const client = createProductApiClient({ fetch: fetchMock });

    const download = await client.exportSessionEvidence("session/safe", format);

    expect(fetchMock).toHaveBeenCalledWith(
      `/api/product/sessions/session%2Fsafe/export?format=${format}`,
      { method: "POST", cache: "no-store" },
    );
    expect(download.filename).toBe(`rove-session-safe-evidence.${extension}`);
    expect(download.mediaType).toBe(`${mediaType}; charset=utf-8`);
    expect(await download.content.text()).toBe(`evidence-${format}`);
  });

  it("rejects a response whose media type contradicts the requested format", async () => {
    const client = createProductApiClient({
      fetch: vi.fn(async () => new Response("not html", {
        status: 200,
        headers: { "content-type": "application/json" },
      })),
    });

    await expect(client.exportSessionEvidence("session-1", "html"))
      .rejects.toBeInstanceOf(ProductApiSchemaError);
  });

  it("uses a bounded local filename when disposition is absent or unsafe", async () => {
    const format: ProductExportFormat = "markdown";
    const client = createProductApiClient({
      fetch: vi.fn(async () => new Response("evidence", {
        status: 200,
        headers: {
          "content-type": "text/markdown",
          "content-disposition": "attachment; filename=../../escape.md",
        },
      })),
    });

    const download = await client.exportSessionEvidence("session/../../secret", format);

    expect(download.filename).toBe("rove-session-session-secret-evidence.md");
    expect(() => downloadEvidenceFile(download)).toThrow(/browser environment/i);
  });
});
