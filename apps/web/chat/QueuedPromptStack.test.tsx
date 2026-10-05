import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { CopyProvider } from "../copy/CopyProvider";
import type { ProductMessage, ProductMessageStatus } from "../product/product-api-types";
import { QueuedPromptStack } from "./QueuedPromptStack";

function renderStack(element: React.ReactElement) {
  return renderToStaticMarkup(<CopyProvider>{element}</CopyProvider>);
}

describe("QueuedPromptStack", () => {
  it("renders only actionable ledger messages beside canonical history", () => {
    const statuses: ProductMessageStatus[] = [
      "queued",
      "intervention_requested",
      "applied_current_run",
      "claimed_successor",
      "needs_attention",
      "revoked",
    ];
    const messages: ProductMessage[] = statuses.map((status, index) => ({
      id: `message-${index + 1}`,
      product_session_id: "session-1",
      content: `ledger-${status}`,
      requested_delivery: status === "intervention_requested" || status === "applied_current_run"
        ? "current_run"
        : "successor",
      status,
      seq: index + 1,
      created_at: "2026-08-14T00:00:00Z",
    }));

    const html = renderStack(<QueuedPromptStack messages={messages} />);

    expect(html).toContain("ledger-queued");
    expect(html).toContain("ledger-intervention_requested");
    expect(html).toContain("ledger-needs_attention");
    expect(html).not.toContain("ledger-applied_current_run");
    expect(html).not.toContain("ledger-claimed_successor");
    expect(html).not.toContain("ledger-revoked");
  });

  it("renders nothing when the ledger holds no actionable rows", () => {
    const html = renderStack(<QueuedPromptStack messages={[]} />);
    expect(html).toBe("");
  });

  it("offers the retry only for a successor the confirm route accepts", () => {
    const stranded: ProductMessage = {
      id: "message-stranded",
      product_session_id: "session-1",
      content: "stranded-successor",
      requested_delivery: "successor",
      status: "needs_attention",
      seq: 1,
      created_at: "2026-09-26T00:00:00Z",
      reason: "successor start failed 3 times",
    };
    const interjection: ProductMessage = {
      ...stranded,
      id: "message-interjection",
      content: "dropped-interjection",
      requested_delivery: "current_run",
    };

    const wired = renderStack(
      <QueuedPromptStack
        messages={[stranded, interjection]}
        onConfirmStranded={vi.fn()}
      />,
    );
    // A stranded successor gets the action; a dropped interjection is a steer
    // the same route refuses, so it gets none.
    expect(wired.match(/重试这条消息/gu)).toHaveLength(1);
    expect(wired).toContain("stranded-successor");
    expect(wired).toContain("dropped-interjection");
    // Nothing is reported before the server has answered.
    expect(wired).not.toContain("queued-message__recovery");

    // A shell without the confirm route offers no recovery action at all.
    const unwired = renderStack(
      <QueuedPromptStack messages={[stranded, interjection]} />,
    );
    expect(unwired).not.toContain("重试这条消息");
  });

  it("orders the successor queue by queue_order and offers both delivery intents", () => {
    const actions = vi.fn();
    const messages: ProductMessage[] = [
      // The ledger seq says "created second"; the explicit queue position says
      // "dispatched first", and the dispatch order is what the row must show.
      {
        id: "message-late",
        product_session_id: "session-1",
        content: "queued-second-in-ledger",
        requested_delivery: "successor",
        status: "queued",
        seq: 2,
        queue_order: 0,
        created_at: "2026-08-14T00:00:00Z",
      },
      {
        id: "message-early",
        product_session_id: "session-1",
        content: "queued-first-in-ledger",
        requested_delivery: "successor",
        status: "queued",
        seq: 1,
        queue_order: 1,
        created_at: "2026-08-14T00:00:00Z",
      },
      {
        id: "message-steer",
        product_session_id: "session-1",
        content: "steer-current-run",
        requested_delivery: "current_run",
        status: "queued",
        seq: 3,
        created_at: "2026-08-14T00:00:00Z",
      },
    ];

    const html = renderStack(
      <QueuedPromptStack
        messages={messages}
        canPromote
        onPromote={actions}
      />,
    );

    // Reordered in the DOM, so adjacency matches the server's dispatch order.
    expect(html.indexOf("queued-second-in-ledger")).toBeLessThan(
      html.indexOf("queued-first-in-ledger"),
    );
    // Both intents are labelled, and the interrupting one says it interrupts.
    expect(html).toContain("立即发送");
    expect(html).toContain("插话");
    expect(html).toContain("不打断当前回合");
    expect(html).toContain("会打断模型正在进行的一步");

    // An interjection is not part of the successor queue, so it exposes neither
    // a reorder control nor a queue position: reordering it is refused server side.
    const steerSlice = html.slice(html.indexOf("steer-current-run"));
    expect(steerSlice).not.toContain("上移");
    expect(steerSlice).not.toContain("下移");
  });

  it("marks the queue head as next-up and no other row", () => {
    const messages: ProductMessage[] = [
      {
        id: "message-next",
        product_session_id: "session-1",
        content: "next up",
        requested_delivery: "successor",
        status: "queued",
        seq: 1,
        created_at: "2026-08-14T00:00:00Z",
      },
      {
        id: "message-later",
        product_session_id: "session-1",
        content: "later",
        requested_delivery: "successor",
        status: "queued",
        seq: 2,
        created_at: "2026-08-14T00:00:00Z",
      },
      {
        id: "message-steer",
        product_session_id: "session-1",
        content: "joining the turn",
        requested_delivery: "current_run",
        status: "intervention_requested",
        seq: 3,
        created_at: "2026-08-14T00:00:00Z",
      },
    ];

    const html = renderStack(<QueuedPromptStack messages={messages} />);
    expect(html.match(/data-next-up/gu)).toHaveLength(1);
    const marker = html.indexOf("data-next-up");
    const nextRow = html.slice(Math.max(0, marker - 400), marker + 400);
    expect(nextRow).toContain("next up");
  });
});
