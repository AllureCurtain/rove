// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CopyProvider } from "../copy/CopyProvider";
import type { ProductSessionCompaction } from "../product/product-api-types";
import { ProductApiError } from "../product/product-client";
import { CompactionPanel } from "./CompactionPanel";

/**
 * Manual compaction panel (R9).
 *
 * Mounted in jsdom because every interesting state exists only after the request
 * resolves. The assertions are about honesty: a degraded compaction must not
 * look like a normal one, a bounded excerpt must not look like the whole
 * summary, a refusal must not be retried, and no provider-supplied string may
 * reach the render.
 */

const SESSION = "01J00000000000000000000002";

function response(
  overrides: Partial<ProductSessionCompaction> = {},
): ProductSessionCompaction {
  return {
    product_session_id: SESSION,
    runtime_run_id: "01J00000000000000000000006",
    triggered: true,
    mode: "model_generated",
    degraded: false,
    consecutive_failures: 0,
    circuit_open: false,
    source_message_count: 12,
    summary_truncated: false,
    token_estimate: 900,
    ...overrides,
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

async function mount(options: {
  responses?: Array<ProductSessionCompaction | Error>;
  sessionId?: string | null;
  sessionStatus?: string | null;
  busy?: boolean;
} = {}) {
  const queue = [...(options.responses ?? [response()])];
  const compactSession = vi.fn(async () => {
    const next = queue.shift();
    if (next instanceof Error) {
      throw next;
    }
    return next ?? response();
  });
  const render = async (overrides: {
    sessionId?: string | null;
    sessionStatus?: string | null;
    busy?: boolean;
  } = {}) => {
    await act(async () => {
      root.render(
        <CopyProvider>
          <CompactionPanel
            sessionId={
              overrides.sessionId === undefined ? SESSION : overrides.sessionId
            }
            sessionStatus={overrides.sessionStatus ?? "idle"}
            busy={overrides.busy ?? false}
            client={{ compactSession }}
          />
        </CopyProvider>,
      );
    });
  };
  await render({
    sessionId: options.sessionId,
    sessionStatus: options.sessionStatus,
    busy: options.busy,
  });
  return { compactSession, render };
}

function button(): HTMLButtonElement {
  const found = container.querySelector("button");
  if (!found) {
    throw new Error("no compaction button rendered");
  }
  return found;
}

async function press() {
  await act(async () => {
    button().click();
  });
}

describe("CompactionPanel", () => {
  it("runs the manual compaction at idle and renders bounded facts", async () => {
    const harness = await mount({ responses: [response({ summary: "excerpt" })] });

    expect(container.textContent).toContain("手动压缩上下文");
    await press();

    expect(harness.compactSession).toHaveBeenCalledTimes(1);
    expect(harness.compactSession).toHaveBeenCalledWith(SESSION);
    expect(container.textContent).toContain("上下文已压缩。");
    expect(container.textContent).toContain("压缩方式：模型摘要");
    expect(container.textContent).toContain("压缩后上下文估计：900 tokens");
    expect(container.textContent).toContain("参与压缩的消息：12 条");
    expect(container.textContent).toContain("excerpt");
    expect(container.textContent).not.toContain("不是完整摘要");
  });

  it("offers the action only at idle and never calls the endpoint while running", async () => {
    const harness = await mount({ busy: true });

    expect(button().disabled).toBe(true);
    expect(container.textContent).toContain("会话空闲时才能手动压缩");
    await press();
    expect(harness.compactSession).not.toHaveBeenCalled();

    await harness.render({ sessionStatus: "running", busy: false });
    expect(button().disabled).toBe(true);
    await press();
    expect(harness.compactSession).not.toHaveBeenCalled();
  });

  it("offers nothing but a reason when no session is open", async () => {
    const harness = await mount({ sessionId: null });

    expect(button().disabled).toBe(true);
    await press();
    expect(harness.compactSession).not.toHaveBeenCalled();
  });

  it("does not present a no-op as a compaction", async () => {
    await mount({ responses: [response({ triggered: false, mode: "none" })] });
    await press();

    expect(container.textContent).toContain("本次没有产生新的摘要");
    expect(container.textContent).not.toContain("上下文已压缩。");
  });

  it("reports a session with no run as nothing to compact", async () => {
    await mount({
      responses: [
        response({ triggered: false, mode: "none", runtime_run_id: undefined }),
      ],
    });
    await press();

    expect(container.textContent).toContain("该会话还没有运行记录");
    expect(container.textContent).not.toContain("上下文已压缩。");
  });

  it("marks a degraded compaction and shows only the typed failure code", async () => {
    await mount({
      responses: [
        response({
          mode: "degraded",
          degraded: true,
          failure_code: "summary_timeout",
        }),
      ],
    });
    await press();

    expect(container.textContent).toContain("本次压缩已降级");
    expect(container.textContent).toContain("summary_timeout");
    expect(container.textContent).toContain("确定性兜底摘要");
  });

  it("still marks a degraded compaction when the API sent no code", async () => {
    await mount({ responses: [response({ degraded: true, mode: "deterministic" })] });
    await press();

    expect(container.textContent).toContain("本次压缩已降级");
    expect(container.textContent).toContain("确定性兜底摘要");
  });

  it("labels a bounded summary as a fragment", async () => {
    await mount({
      responses: [
        response({ summary: "the beginning only", summary_truncated: true }),
      ],
    });
    await press();

    expect(container.textContent).toContain("the beginning only");
    expect(container.textContent).toContain("不是完整摘要");
  });

  it("describes the open breaker as persisted history, with the window only when sent", async () => {
    await mount({
      responses: [
        response({
          circuit_open: true,
          consecutive_failures: 3,
          triggered: false,
          mode: "deterministic",
        }),
      ],
    });
    await press();

    expect(container.textContent).toContain("摘要模型已连续失败 3 次");
    expect(container.textContent).toContain("手动压缩仍会做一次探针");
    expect(container.textContent).not.toContain("之前会被拒绝");
  });

  it("states the refusal window when the API sent one", async () => {
    await mount({
      responses: [
        response({
          circuit_open: true,
          consecutive_failures: 2,
          next_attempt_after: "2026-09-26T00:05:00.000Z",
        }),
      ],
    });
    await press();

    expect(container.textContent).toContain(
      "自动压缩在 2026-09-26T00:05:00.000Z 之前会被拒绝。",
    );
  });

  it("renders a mode this build does not know verbatim instead of guessing", async () => {
    await mount({ responses: [response({ mode: "hybrid_preview" })] });
    await press();

    expect(container.textContent).toContain("压缩方式：hybrid_preview");
  });

  it("reports a running session as a refusal and does not retry it", async () => {
    const harness = await mount({
      responses: [
        new ProductApiError(409, "product_session_active", "session is running"),
      ],
    });
    await press();

    expect(container.textContent).toContain("会话仍在运行，未执行压缩");
    // The endpoint takes the same claim a turn takes and does not queue behind
    // it, so a retry would only be refused again.
    expect(harness.compactSession).toHaveBeenCalledTimes(1);
  });

  it("reports a typed failure without inventing a cause", async () => {
    const harness = await mount({
      responses: [new ProductApiError(500, "product_internal_error", "boom")],
    });
    await press();

    expect(container.textContent).toContain("product_internal_error");
    // The provider's own message is never in the response, and even a stray one
    // must not reach the render.
    expect(container.textContent).not.toContain("boom");
    expect(harness.compactSession).toHaveBeenCalledTimes(1);
  });

  it("clears a result that belongs to a session the user left", async () => {
    const harness = await mount({ responses: [response({ summary: "old excerpt" })] });
    await press();
    expect(container.textContent).toContain("old excerpt");

    await harness.render({ sessionId: "01J00000000000000000000009" });

    expect(container.textContent).not.toContain("old excerpt");
    expect(container.textContent).not.toContain("上下文已压缩。");
  });

  it("ignores a second press while the first call is in flight", async () => {
    let release: ((value: ProductSessionCompaction) => void) | null = null;
    const compactSession = vi.fn(
      () =>
        new Promise<ProductSessionCompaction>((resolve) => {
          release = resolve;
        }),
    );
    await act(async () => {
      root.render(
        <CopyProvider>
          <CompactionPanel
            sessionId={SESSION}
            sessionStatus="idle"
            busy={false}
            client={{ compactSession }}
          />
        </CopyProvider>,
      );
    });

    await press();
    expect(container.textContent).toContain("正在压缩上下文…");
    await press();

    expect(compactSession).toHaveBeenCalledTimes(1);
    await act(async () => {
      release?.(response());
    });
    expect(container.textContent).toContain("上下文已压缩。");
  });
});
