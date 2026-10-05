// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CopyProvider } from "../copy/CopyProvider";
import { ProductApiError } from "../product/product-client";
import type { ProductSearchResponse } from "../product/product-api-types";
import { ProductSearchView } from "./ProductSearchView";

/**
 * Unified search view (R7 step 2).
 *
 * Mounted in jsdom because every interesting state here is asynchronous: the
 * partial-page note, the expired-cursor restart, and the refused input all exist
 * only after a request resolves. `renderToStaticMarkup` never re-renders, so it
 * cannot observe them.
 */

const SCOPE = "session:s1";

function page(overrides: Partial<ProductSearchResponse> = {}): ProductSearchResponse {
  return { scope: SCOPE, hits: [], ...overrides };
}

function messageHit(seq: number, snippet: string) {
  return {
    session_id: "s1",
    source: "message" as const,
    seq,
    snippet,
    created_at: "2026-09-26T00:00:00.000Z",
  };
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

interface Harness {
  searchProduct: ReturnType<typeof vi.fn>;
  onOpenSession: ReturnType<typeof vi.fn>;
  onLocateMessage: ReturnType<typeof vi.fn>;
  onClose: ReturnType<typeof vi.fn>;
}

async function mount(options: {
  responses?: Array<ProductSearchResponse | Error>;
  locate?: "located" | "not_loaded";
} = {}): Promise<Harness> {
  const queue = [...(options.responses ?? [])];
  const searchProduct = vi.fn(async () => {
    const next = queue.shift();
    if (next instanceof Error) {
      throw next;
    }
    return next ?? page();
  });
  const onOpenSession = vi.fn();
  const onLocateMessage = vi.fn(() => options.locate ?? "located");
  const onClose = vi.fn();
  await act(async () => {
    root.render(
      <CopyProvider>
        <ProductSearchView
          open
          sessionId="s1"
          workspaceId="w1"
          sessions={[{ id: "s1", title: "First session" }]}
          client={{ searchProduct }}
          onOpenSession={onOpenSession}
          onLocateMessage={onLocateMessage}
          onClose={onClose}
        />
      </CopyProvider>,
    );
  });
  return { searchProduct, onOpenSession, onLocateMessage, onClose };
}

async function type(value: string) {
  const input = container.querySelector<HTMLInputElement>("#product-search-term")!;
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

async function selectScope(kind: string) {
  const select = container.querySelector<HTMLSelectElement>("#product-search-scope")!;
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(
      HTMLSelectElement.prototype,
      "value",
    )!.set!;
    setter.call(select, kind);
    select.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

describe("ProductSearchView", () => {
  it("renders a snippet verbatim with its source and session title", async () => {
    const harness = await mount({
      responses: [page({ hits: [messageHit(4, "…alpha needle omega…")] })],
    });
    await type("needle");
    await submit();

    expect(harness.searchProduct).toHaveBeenCalledTimes(1);
    expect(harness.searchProduct.mock.calls[0]![0]).toMatchObject({
      q: "needle",
      scope: SCOPE,
      limit: 32,
    });
    expect(container.textContent).toContain("…alpha needle omega…");
    expect(container.textContent).toContain("First session");
    expect(container.textContent).toContain("消息");
  });

  it("shows the empty state and no partial note for a complete empty page", async () => {
    await mount({ responses: [page({ hits: [] })] });
    await type("needle");
    await submit();

    expect(container.textContent).toContain("没有匹配结果。");
    expect(container.textContent).not.toContain("本次只读完了一部分事件记录");
    expect(container.textContent).toContain("已到最后一页。");
  });

  it("marks a short page that still has a cursor as a bounded read", async () => {
    await mount({
      responses: [page({ hits: [messageHit(1, "one")], next_cursor: "c1" })],
    });
    await type("needle");
    await submit();

    expect(container.textContent).toContain("本次只读完了一部分事件记录");
    expect(container.textContent).toContain("继续加载");
    expect(container.textContent).not.toContain("已到最后一页。");
  });

  it("renders a typed failure without inventing a cause", async () => {
    await mount({
      responses: [new ProductApiError(400, "product_invalid_input", "bad term")],
    });
    await type("needle");
    await submit();

    expect(container.textContent).toContain("product_invalid_input");
    expect(container.textContent).not.toContain("没有匹配结果。");
  });

  it("drops an expired cursor and restarts from the first page once", async () => {
    const harness = await mount({
      responses: [
        page({ hits: [messageHit(1, "first page")], next_cursor: "c1" }),
        new ProductApiError(409, "product_events_expired", "gone"),
        page({ hits: [messageHit(2, "after restart")] }),
      ],
    });
    await type("needle");
    await submit();
    // The expiry only exists once a cursor was in play, so the cursor has to be
    // obtained through a real "load more" rather than assumed.
    const loadMore = [...container.querySelectorAll("button")].find(
      (button) => button.textContent === "继续加载",
    );
    expect(loadMore).toBeTruthy();
    await act(async () => {
      loadMore!.click();
    });

    expect(harness.searchProduct).toHaveBeenCalledTimes(3);
    expect(harness.searchProduct.mock.calls[0]![0]).not.toHaveProperty("cursor");
    expect(harness.searchProduct.mock.calls[1]![0]).toHaveProperty("cursor", "c1");
    // The restart may reuse neither the term-less request nor the dead cursor:
    // it repeats the same term with no cursor at all.
    expect(harness.searchProduct.mock.calls[2]![0]).not.toHaveProperty("cursor");
    expect(harness.searchProduct.mock.calls[2]![0]).toHaveProperty("q", "needle");
    expect(container.textContent).toContain("after restart");
    expect(container.textContent).not.toContain("没有匹配结果。");
    // The dropped cursor stays visible after the new page loads, so the user is
    // not shown a page that silently replaced the one they asked for.
    expect(container.textContent).toContain("已丢弃游标从第一页重新开始");
  });

  it("reports a cleaned trace page as content that no longer exists", async () => {
    const harness = await mount({
      responses: [new ProductApiError(409, "product_events_expired", "cleaned")],
    });
    await selectScope("trace");
    await type("needle");
    await submit();

    // A first page has no cursor to drop, so the view reports instead of looping.
    expect(harness.searchProduct).toHaveBeenCalledTimes(1);
    expect(container.textContent).toContain("已丢弃游标从第一页重新开始");
    expect(container.textContent).not.toContain("这些 run 的事件记录里没有匹配内容。");
  });

  it("refuses a workspace term the scope cannot answer", async () => {
    const harness = await mount();
    await selectScope("workspace");
    await type("ab");
    await submit();

    expect(harness.searchProduct).not.toHaveBeenCalled();
    expect(container.textContent).toContain("工作区范围需要至少 3 个字符");
  });

  it("refuses a blank term", async () => {
    const harness = await mount();
    await type("   ");
    await submit();

    expect(harness.searchProduct).not.toHaveBeenCalled();
    expect(container.textContent).toContain("请输入关键词。");
  });

  it("locates a message hit through the host and reports the answer", async () => {
    const harness = await mount({
      responses: [page({ hits: [messageHit(7, "hit")] })],
      locate: "not_loaded",
    });
    await type("needle");
    await submit();

    const locateButton = [...container.querySelectorAll("button")].find(
      (button) => button.textContent === "定位到该消息",
    );
    expect(locateButton).toBeTruthy();
    await act(async () => {
      locateButton!.click();
    });

    expect(harness.onLocateMessage).toHaveBeenCalledWith(7);
    expect(container.textContent).toContain("该消息不在已加载的记录范围内");
  });

  it("never sends a trace hit's sequence to the locator", async () => {
    const harness = await mount({
      responses: [
        page({
          // The server echoes the canonical scope it resolved; a page that
          // echoed the session scope here would not belong to this search.
          scope: "trace:s1",
          hits: [
            {
              session_id: "s1",
              source: "trace",
              seq: 12,
              run_ordinal: 3,
              snippet: '{"type":"run_started"}',
            },
          ],
        }),
      ],
    });
    await selectScope("trace");
    await type("needle");
    await submit();

    const openButton = [...container.querySelectorAll("button")].find(
      (button) => button.textContent === "打开会话",
    );
    expect(openButton).toBeTruthy();
    await act(async () => {
      openButton!.click();
    });
    expect(harness.onLocateMessage).not.toHaveBeenCalled();
    expect(harness.onOpenSession).toHaveBeenCalledWith("s1");
    // An absent timestamp is reported as absent, never filled in.
    expect(container.textContent).toContain("无时间戳");
    expect(container.textContent).toContain("run 3");
  });
});
