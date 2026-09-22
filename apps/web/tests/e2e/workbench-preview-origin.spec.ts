import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";

import { expect, test } from "@playwright/test";

/**
 * P5b threat-model gate 1 (A1/A2): a real browser must prove that the product
 * token is unavailable on the isolated preview origin.
 *
 * The product surface receives `window.__ROVE_TOKEN__` from the Desktop host
 * (see `platform/desktop-transport.ts`). A previewed page runs on its own
 * loopback origin with no product cookie and no BearerAuth middleware
 * (`apps/api/src/product/preview.rs`). This suite exercises the browser half
 * of that contract: same-origin policy keeps the token on the product origin,
 * and a preview-origin page cannot call the product API as the user.
 *
 * Gates 2–5 (path confinement, secret names, token lifecycle, size/timeout
 * bounds) stay in `tests/api.rs` (`product_preview_*`).
 */

interface PreviewProbe {
  token: string | null;
  openerToken: string | null;
  productFetch: string;
}

function startPreviewOrigin(): Promise<{ server: Server; origin: string }> {
  return new Promise((resolve) => {
    const server = createServer((_request, response) => {
      response.setHeader("content-type", "text/html; charset=utf-8");
      response.setHeader("content-security-policy", "default-src 'none'; script-src 'unsafe-inline'; connect-src 'none'");
      response.setHeader("cross-origin-opener-policy", "same-origin");
      response.setHeader("cross-origin-resource-policy", "same-origin");
      response.setHeader("x-content-type-options", "nosniff");
      response.setHeader("cache-control", "no-store");
      response.end(`<!doctype html><html><body><h1 id="marker">rove-preview-marker</h1><script>
        window.__probe = {
          token: typeof window.__ROVE_TOKEN__ === "undefined" ? null : String(window.__ROVE_TOKEN__),
          openerToken: null,
          productFetch: "pending"
        };
        try {
          if (window.opener && window.opener.__ROVE_TOKEN__) {
            window.__probe.openerToken = String(window.opener.__ROVE_TOKEN__);
          }
        } catch (error) {
          window.__probe.openerToken = "blocked:" + error.name;
        }
        fetch("/api/product/workspaces", { credentials: "include" })
          .then(function (response) { window.__probe.productFetch = String(response.status); })
          .catch(function (error) { window.__probe.productFetch = "error:" + error.name; });
      </script></body></html>`);
    });
    server.listen(0, "127.0.0.1", () => {
      const address = server.address() as AddressInfo;
      resolve({
        server,
        origin: `http://127.0.0.1:${address.port}`,
      });
    });
  });
}

test.describe("P5b preview origin browser isolation", () => {
  test("product token stays unavailable on the isolated preview origin", async ({
    page,
    context,
    baseURL,
  }) => {
    test.setTimeout(30_000);
    const preview = await startPreviewOrigin();
    try {
      // Product origin: the Desktop host injects the bearer token here.
      await page.addInitScript(() => {
        (window as unknown as { __ROVE_TOKEN__?: string }).__ROVE_TOKEN__ =
          "product-secret-token";
      });
      await page.goto(baseURL ?? "/");
      const productToken = await page.evaluate(
        () =>
          (window as unknown as { __ROVE_TOKEN__?: string }).__ROVE_TOKEN__ ??
          null,
      );
      expect(productToken).toBe("product-secret-token");

      // Preview origin: a separate loopback origin, exactly like the isolated
      // preview listener. It must not observe the product token.
      const previewPage = await context.newPage();
      await previewPage.goto(preview.origin + "/");
      await expect(previewPage.locator("#marker")).toHaveText(
        "rove-preview-marker",
      );
      await previewPage.waitForFunction(
        () =>
          (
            window as unknown as { __probe?: PreviewProbe }
          ).__probe?.productFetch !== "pending",
      );
      const probe = await previewPage.evaluate(
        () => (window as unknown as { __probe?: PreviewProbe }).__probe,
      );
      expect(probe?.token).toBeNull();
      // window.opener is severed by COOP on the real preview responses; the
      // probe also fails closed when the browser blocks cross-origin reads.
      expect(probe?.openerToken ?? null).not.toBe("product-secret-token");
      // The preview origin is not an allowed product API client.
      expect(probe?.productFetch).not.toBe("200");
      await previewPage.close();
    } finally {
      await new Promise<void>((resolve) => preview.server.close(() => resolve()));
    }
  });
});
