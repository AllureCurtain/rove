// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CopyProvider } from "../copy/CopyProvider";
import { ProductApiError } from "../product/product-client";
import { SessionMessageSearchPanel } from "./SessionMessageSearchPanel";

/**
 * Session message search (R7 step 1).
 *
 * The endpoint's quiet failure modes are what these cases pin down: zero hits is
 * only ever a successful empty page, a refusal and a missing session are their
 * own states, a cursor is never reused across a changed term, and the excerpt is
 * rendered exactly as sent.
 */

const SESSION = "01J00000000000000000000002";

function hit(seq: number, snippet: string) {
  return { message_seq: seq, snippet, created_at: "2026-09-26T00:00:00.000Z" };
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => {
    root.unmount();
  });
  container.remove();
});

async function mount(options: {
  responses?: Array<{ hits: ReturnType<typeof hit>[]; next_cursor?: string } | Error>;
  sessionId?: string | null;
  locate?: "located" | "not_loaded";
} = {}) {
  const queue = [...(options.responses ?? [{ hits: [] }])];
  const searchSessionMessages = vi.fn(
    async (_sessionId: string, _query: { q: string; cursor?: string; limit?: number }) => {
      const next = queue.shift();
      if (next instanceof Error) {
        throw next;
      }
      return next ?? { hits: [] };
    },
  );
  const onLocateMessage = vi.fn(() => options.locate ?? "located");
  const onClose = vi.fn();
  await act(async () => {
    root.render(
      <CopyProvider>
        <SessionMessageSearchPanel
          open
          sessionId={options.sessionId === undefined ? SESSION : options.sessionId}
          client={{ searchSessionMessages }}
          onLocateMessage={onLocateMessage}
          onClose={onClose}
        />
      </CopyProvider>,
    );
  });
  return { searchSessionMessages, onLocateMessage, onClose };
}

async function type(value: string) {
  const input = container.querySelector<HTMLInputElement>(
    "#session-message-search-term",
  )!;
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!;
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function submit() {
  const form = container.querySelector("form")!;
  await act(async () => {
    form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  });
}

describe("SessionMessageSearchPanel", () => {
  it("searches the open session and renders the excerpt verbatim", async () => {
    const harness = await mount({
      responses: [
        {
          hits: [
            hit(7, "…[REDACTED:secret_pattern] appears here…"),
          ],
        },
      ],
    });

    await type("needle");
    await submit();

    expect(harness.searchSessionMessages).toHaveBeenCalledWith(SESSION, {
      q: "needle",
      limit: 32,
    });
    // The server bounded and redacted this window; re-cutting it would show
    // something the API did not send.
    expect(container.textContent).toContain("…[REDACTED:secret_pattern] appears here…");
    expect(container.textContent).toContain("已到最后一页。");
  });

  it("shows an empty successful page as no matching message", async () => {
    await mount({ responses: [{ hits: [] }] });
    await type("needle");
    await submit();

    expect(container.textContent).toContain("本会话没有匹配的消息。");
  });

  it("does not present a refused request as an empty result", async () => {
    await mount({
      responses: [new ProductApiError(400, "product_invalid_input", "bad cursor")],
    });
    await type("needle");
    await submit();

    expect(container.textContent).toContain("product_invalid_input");
    expect(container.textContent).not.toContain("本会话没有匹配的消息。");
  });

  it("does not present a missing session as an empty result", async () => {
    await mount({
      responses: [new ProductApiError(404, "product_not_found", "gone")],
    });
    await type("needle");
    await submit();

    expect(container.textContent).toContain("product_not_found");
    expect(container.textContent).not.toContain("本会话没有匹配的消息。");
  });

  it("refuses a blank term before the request leaves", async () => {
    const harness = await mount();
    await type("   ");
    await submit();

    expect(harness.searchSessionMessages).not.toHaveBeenCalled();
    expect(container.textContent).toContain("至少需要 1 个字符");
  });

  it("refuses a term carrying control characters before the request leaves", async () => {
    const harness = await mount();
    await type(`needle${String.fromCharCode(7)}`);
    await submit();

    expect(harness.searchSessionMessages).not.toHaveBeenCalled();
    expect(container.textContent).toContain("控制字符");
  });

  it("counts the term limit in bytes", async () => {
    const harness = await mount();
    await type("测".repeat(43));
    await submit();

    expect(harness.searchSessionMessages).not.toHaveBeenCalled();
    expect(container.textContent).toContain("最多 128 字节");
  });

  it("drops the cursor when the term changes instead of reusing it", async () => {
    const harness = await mount({
      responses: [{ hits: [hit(1, "first")], next_cursor: "c1" }, { hits: [hit(2, "second")] }],
    });
    await type("needle");
    await submit();

    const loadMore = [...container.querySelectorAll("button")].find(
      (button) => button.textContent === "继续加载",
    );
    expect(loadMore).toBeTruthy();

    // A changed term invalidates the cursor the first page issued.
    await type("different");
    await act(async () => {
      loadMore!.click();
    });

    expect(harness.searchSessionMessages).toHaveBeenCalledTimes(1);

    // Resubmitting asks the first page of the new term, with no cursor.
    await submit();
    expect(harness.searchSessionMessages).toHaveBeenCalledTimes(2);
    expect(harness.searchSessionMessages.mock.calls[1]![1]).toEqual({
      q: "different",
      limit: 32,
    });
  });

  it("appends the next page when the term is unchanged", async () => {
    const harness = await mount({
      responses: [{ hits: [hit(1, "first")], next_cursor: "c1" }, { hits: [hit(2, "second")] }],
    });
    await type("needle");
    await submit();

    const loadMore = [...container.querySelectorAll("button")].find(
      (button) => button.textContent === "继续加载",
    );
    await act(async () => {
      loadMore!.click();
    });

    expect(harness.searchSessionMessages.mock.calls[1]![1]).toEqual({
      q: "needle",
      cursor: "c1",
      limit: 32,
    });
    expect(container.textContent).toContain("first");
    expect(container.textContent).toContain("second");
  });

  it("renders the locator's answer instead of claiming a jump", async () => {
    const harness = await mount({
      responses: [{ hits: [hit(9, "hit")] }],
      locate: "not_loaded",
    });
    await type("needle");
    await submit();

    const locate = [...container.querySelectorAll("button")].find(
      (button) => button.textContent === "定位到该消息",
    );
    await act(async () => {
      locate!.click();
    });

    expect(harness.onLocateMessage).toHaveBeenCalledWith(9);
    expect(container.textContent).toContain("该消息不在已加载的记录范围内");
  });

  it("asks for a session first instead of searching without one", async () => {
    const harness = await mount({ sessionId: null });
    await type("needle");
    await submit();

    expect(harness.searchSessionMessages).not.toHaveBeenCalled();
    expect(container.textContent).toContain("需要先打开一个会话。");
  });
});
