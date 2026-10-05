import { expect, test, type Locator, type Page } from "@playwright/test";

import { installMockProductApi } from "./product-api-mock";

/**
 * R8 / F13: the composer uploads through the same authenticated transport as
 * every other product call, and the transcript renders the references the
 * server stored instead of anything the browser remembered.
 *
 * The real route sniffs bytes and owns the caps; these cases hold the client to
 * the same vocabulary — an indeterminate upload, the server's confirmation,
 * three warning codes, a typed refusal per failure class, and a reference that
 * is fetched rather than linked.
 */

const PNG_BYTES = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8AAAwAB/wD/AL0AAAAASUVORK5CYII=",
  "base64",
);

const PDF = { name: "brief.pdf", mimeType: "application/pdf", buffer: Buffer.from("brief") };

async function openComposer(
  page: Page,
  options?: Parameters<typeof installMockProductApi>[1],
) {
  const api = await installMockProductApi(page, options);
  await page.goto("/");
  await page.getByText("手动输入路径").click();
  await page.getByLabel("绝对路径").fill("D:/tmp/rove-attachments");
  await page.getByRole("button", { name: "打开工作区", exact: true }).click();
  const composer = page.getByRole("textbox", { name: /输入消息/ });
  await expect(composer).toBeEnabled();
  return {
    api,
    composer,
    send: page.getByRole("button", { name: "发送", exact: true }),
    stop: page.getByRole("button", { name: "停止", exact: true }),
  };
}

/** The hidden input is the only way a real user's picker reaches the composer. */
async function attach(
  page: Page,
  file: { name: string; mimeType: string; buffer: Buffer },
) {
  await page.locator('input[type="file"]').setInputFiles(file);
}

function rows(page: Page): Locator {
  return page.locator(".file-chip");
}

test("an attached image uploads, sends, and comes back as a fetched reference", async ({
  page,
}) => {
  const { api, composer, send } = await openComposer(page);

  await composer.fill("look at this");
  await attach(page, { name: "shot.png", mimeType: "image/png", buffer: PNG_BYTES });

  const row = rows(page).first();
  await expect(row).toBeVisible();
  await expect(row.locator(".file-chip__name")).toHaveText("shot.png");
  await expect(row).toHaveAttribute("data-status", "uploaded");
  await expect(row.locator(".file-chip__meta")).toContainText("已就绪");
  await expect(send).toBeEnabled();
  // The bytes went out as the raw body of an authenticated POST that named the
  // file, not as a multipart envelope and not as a bare link.
  expect(api.attachmentUploads).toEqual([
    {
      sessionId: expect.any(String),
      name: "shot.png",
      contentType: "image/png",
      bodyBytes: PNG_BYTES.byteLength,
    },
  ]);

  await composer.press("Control+Enter");
  await expect(composer).toHaveValue("");
  expect(api.jobs).toHaveLength(1);

  // The transcript renders the reference from the durable ledger row, keyed by
  // the run that carried it.
  const reference = page.getByRole("button", { name: "查看图片 shot.png" });
  await expect(reference).toBeVisible();
  await expect(reference).toContainText("shot.png");

  await reference.click();
  const image = page.locator(".message-attachment__image");
  await expect(image).toBeVisible();
  // Fetched through the client and shown from an object URL, so a Desktop
  // bearer token would have travelled with it.
  await expect(image).toHaveAttribute("src", /^blob:/);

  // The reference moved to the message; it did not stay attached to the next
  // send.
  await expect(rows(page)).toHaveCount(0);
});

test("an upload stays indeterminate until the server confirms it", async ({ page }) => {
  const { api, composer, send } = await openComposer(page);

  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route("**/api/product/sessions/*/attachments*", async (route) => {
    if (route.request().method() !== "POST") return route.fallback();
    await gate;
    return route.fallback();
  });

  await composer.fill("waiting on the upload");
  await attach(page, { name: "shot.png", mimeType: "image/png", buffer: PNG_BYTES });

  const row = rows(page).first();
  await expect(row).toHaveAttribute("data-status", "uploading");
  await expect(row.locator(".file-chip__meta")).toContainText("正在上传");
  // No percentage is known while the body is in flight, so the surface never
  // claims one.
  await expect(row.locator('[role="progressbar"]')).toHaveAttribute(
    "aria-label",
    "正在上传…",
  );
  // The draft is real, so a disabled send here can only mean the unfinished
  // attachment.
  await expect(send).toBeDisabled();
  await expect(composer).toBeEnabled();

  release();
  await expect(row).toHaveAttribute("data-status", "uploaded");
  await expect(send).toBeEnabled();
  expect(api.attachmentUploads).toHaveLength(1);
});

test("a retryable refusal is retried once, a permanent one is not", async ({ page }) => {
  const { api } = await openComposer(page, {
    attachmentFailures: [
      { status: 429, code: "product_attachment_busy" },
      { status: 413, code: "product_attachment_too_large" },
    ],
  });

  await attach(page, PDF);

  const row = rows(page).first();
  await expect(row).toHaveAttribute("data-status", "failed");
  await expect(row.locator(".file-chip__error")).toContainText("20.0 MiB");
  // The busy refusal was answered by one retry, which minted a fresh upload
  // rather than reusing an id the server had refused.
  expect(api.attachmentUploads).toHaveLength(2);
  // A too-large refusal is not about timing, so no retry is offered: the reader
  // removes the row instead of looping on a file the server will never take.
  await expect(row.getByRole("button", { name: "重试" })).toHaveCount(0);
  await expect(row.getByRole("button", { name: "移除附件 brief.pdf" })).toBeVisible();
});

const refusalClasses = [
  { status: 403, code: "product_attachment_quota", expected: "配额" },
  { status: 415, code: "product_attachment_invalid_input", expected: "类型" },
  { status: 503, code: "product_attachment_unavailable", expected: "不可用" },
  { status: 504, code: "product_attachment_timeout", expected: "超时" },
];

for (const { status, code, expected } of refusalClasses) {
  test(`${code} is shown as its own failure, not as a generic one`, async ({ page }) => {
    const { composer, send } = await openComposer(page, {
      // Two refusals: the automatic retry is refused too, so the row settles.
      attachmentFailures: [
        { status, code },
        { status, code },
      ],
    });
    await composer.fill("send the brief");
    await attach(page, PDF);

    const row = rows(page).first();
    await expect(row).toHaveAttribute("data-status", "failed");
    await expect(row.locator(".file-chip__error")).toContainText(expected);
    await expect(row.locator(".file-chip__error")).toHaveAttribute("role", "alert");
    // An upload that never succeeded carries no reference, so the send stays
    // blocked rather than silently dropping the file.
    await expect(send).toBeDisabled();
  });
}

test("a file the server would refuse is refused locally, without a request", async ({
  page,
}) => {
  const { api, composer } = await openComposer(page);
  await composer.fill("no attachment survives this");

  await attach(page, {
    name: "installer.exe",
    mimeType: "application/octet-stream",
    buffer: Buffer.from("MZ"),
  });

  await expect(page.locator(".chat-composer__paste-notice")).toContainText("类型");
  await expect(rows(page)).toHaveCount(0);
  expect(api.attachmentUploads).toHaveLength(0);
  // The draft itself is untouched by a refused pick.
  await expect(composer).toHaveValue("no attachment survives this");
});

test("a warning the server reports is shown on the row it belongs to", async ({
  page,
}) => {
  const { api } = await openComposer(page, {
    attachmentWarnings: ["secret_shaped_name", "content_type_claim_mismatch"],
  });

  await attach(page, PDF);

  const row = rows(page).first();
  await expect(row).toHaveAttribute("data-status", "uploaded");
  await expect(
    row.locator('.file-chip__warning[data-code="secret_shaped_name"]'),
  ).toContainText("凭据");
  await expect(
    row.locator('.file-chip__warning[data-code="content_type_claim_mismatch"]'),
  ).toContainText("类型");
  expect(api.attachmentUploads).toHaveLength(1);
});

test("an image the model cannot take is shown as degraded, not as sent", async ({
  page,
}) => {
  const { composer } = await openComposer(page, {
    attachmentDegradation: "not sent: the selected model does not accept image input",
  });

  await attach(page, { name: "shot.png", mimeType: "image/png", buffer: PNG_BYTES });
  await expect(rows(page).first()).toHaveAttribute("data-status", "uploaded");
  await composer.fill("look at this");
  await composer.press("Control+Enter");

  const degraded = page.locator('.message-attachment[data-degraded="true"]');
  await expect(degraded).toBeVisible();
  await expect(degraded.locator(".message-attachment__degraded")).toContainText(
    "该图片未发送给模型",
  );
});

test("a non-image reference is saved by fetching it, never linked by URL", async ({
  page,
}) => {
  const { composer } = await openComposer(page);

  await attach(page, PDF);
  await expect(rows(page).first()).toHaveAttribute("data-status", "uploaded");
  await composer.fill("read this");
  await composer.press("Control+Enter");

  const save = page.getByRole("button", { name: "保存附件 brief.pdf" });
  await expect(save).toBeVisible();
  const download = page.waitForEvent("download");
  await save.click();
  expect((await download).suggestedFilename()).toBe("brief.pdf");
});

test("a stopped send restores a reference the server still has", async ({ page }) => {
  const { api, composer, stop, send } = await openComposer(page, {
    mode: "waiting_model",
  });

  await attach(page, { name: "shot.png", mimeType: "image/png", buffer: PNG_BYTES });
  await expect(rows(page).first()).toHaveAttribute("data-status", "uploaded");
  await composer.fill("look at this");
  await composer.press("Control+Enter");
  await expect(stop).toBeVisible();
  await expect(rows(page)).toHaveCount(0);

  await stop.click();
  await expect(composer).toHaveValue("look at this");

  // The row came back from the snapshot without the bytes, so the shell confirms
  // the reference against the server before the draft is sendable again.
  const row = rows(page).first();
  await expect(row).toHaveAttribute("data-restored", "true");
  await expect(row).toHaveAttribute("data-status", "uploaded");
  await expect(row.locator(".file-chip__meta")).toContainText("已恢复");
  await expect(send).toBeEnabled();

  // Resending the restored draft reuses the same reference; it does not upload
  // the bytes a second time.
  //
  // One press, wherever it lands: a press inside the re-read a stop triggers is
  // deferred by the composer and goes out when the read lands, so the reader does
  // not have to press again. The turn count below catches a second send.
  const turnsBefore = api.jobStarts.length;
  await composer.press("Control+Enter");
  await expect(composer).toHaveValue("", { timeout: 20_000 });
  expect(api.jobStarts.length - turnsBefore).toBe(1);
  expect(api.attachmentUploads).toHaveLength(1);
  expect(api.attachmentProbes).toHaveLength(1);
});

test("a restored reference the server no longer has is a re-attach, not a retry", async ({
  page,
}) => {
  const { composer, stop, send } = await openComposer(page, {
    mode: "waiting_model",
    attachmentProbeFailures: {
      "attachment-1": [{ status: 409, code: "product_attachment_conflict" }],
    },
  });

  await attach(page, { name: "shot.png", mimeType: "image/png", buffer: PNG_BYTES });
  await composer.fill("look at this");
  await composer.press("Control+Enter");
  await expect(stop).toBeVisible();
  await stop.click();
  await expect(composer).toHaveValue("look at this");

  const row = rows(page).first();
  await expect(row).toHaveAttribute("data-status", "failed");
  await expect(row.locator(".file-chip__error")).toContainText("已过期");
  // The bytes were never kept after a successful upload, so there is nothing to
  // retry with; the reader removes the dead reference and picks the file again.
  await expect(row.getByRole("button", { name: "重试" })).toHaveCount(0);
  await expect(row.getByRole("button", { name: "移除附件 shot.png" })).toBeVisible();
  await expect(send).toBeDisabled();
});
