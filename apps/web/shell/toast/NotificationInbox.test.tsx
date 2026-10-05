// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CopyProvider } from "../../copy/CopyProvider";
import { NotificationInbox } from "./NotificationInbox";
import { ToastProvider, useToast } from "./ToastProvider";
import type { ToastInput, ToastTarget } from "./toast-store";

/**
 * Notification inbox (design F5).
 *
 * Mounted in jsdom because every interesting state exists only after a toast has
 * been raised through the provider. The assertions are about the three claims
 * the surface makes: it holds this page's failures and nothing else, the badge
 * counts exactly what the list marks unread, and a row only offers navigation it
 * can actually perform.
 */

let container: HTMLDivElement;
let root: Root;
let opened: ToastTarget[];
let raise: (input: ToastInput) => void;
/** The shell's answer for the next row activation: true when it navigated. */
let unresolved: boolean;

function Harness() {
  const toast = useToast();
  // The test raises notifications through this handle rather than reaching into
  // the provider's internals.
  raise = (input: ToastInput) => toast.notify(input);
  return (
    <NotificationInbox
      onOpenTarget={(target) => {
        opened.push(target);
        return !unresolved;
      }}
    />
  );
}

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  // The provider reads the reduced-motion preference on mount; jsdom has no
  // matchMedia, and the inbox does not depend on the answer.
  vi.stubGlobal(
    "matchMedia",
    (query: string) =>
      ({
        matches: false,
        media: query,
        addEventListener: () => undefined,
        removeEventListener: () => undefined,
      }) as unknown as MediaQueryList,
  );
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  opened = [];
  unresolved = false;
  raise = () => undefined;
});

afterEach(async () => {
  await act(async () => {
    root.unmount();
  });
  container.remove();
  vi.unstubAllGlobals();
});

async function mount() {
  await act(async () => {
    root.render(
      <CopyProvider>
        <ToastProvider>
          <Harness />
        </ToastProvider>
      </CopyProvider>,
    );
  });
}

/** jsdom needs a frame before the focus call inside the panel's effect runs. */
async function flushFrame() {
  await act(async () => {
    await new Promise<void>((resolve) => {
      window.requestAnimationFrame(() => resolve());
    });
  });
}

function bell(): HTMLButtonElement {
  const found = container.querySelector<HTMLButtonElement>(".notification-inbox__bell");
  if (!found) {
    throw new Error("no bell rendered");
  }
  return found;
}

function panel(): HTMLElement | null {
  return container.querySelector<HTMLElement>(".notification-inbox__panel");
}

function rows(): HTMLElement[] {
  return Array.from(container.querySelectorAll<HTMLElement>(".notification-inbox__row"));
}

async function notify(input: ToastInput) {
  await act(async () => {
    raise(input);
  });
}

async function clickBell() {
  await act(async () => {
    bell().click();
  });
  await flushFrame();
}

describe("NotificationInbox", () => {
  it("stays out of the way until something fails", async () => {
    await mount();

    expect(bell().getAttribute("aria-expanded")).toBe("false");
    // The bell only names the panel while the panel is there to be named.
    expect(bell().getAttribute("aria-controls")).toBeNull();
    expect(container.querySelector(".notification-inbox__badge")).toBeNull();
    expect(bell().getAttribute("aria-label")).toBe("打开通知收件箱");
    expect(panel()).toBeNull();
  });

  it("says the list is empty and bounded rather than showing nothing", async () => {
    await mount();
    await clickBell();

    expect(panel()).not.toBeNull();
    expect(bell().getAttribute("aria-expanded")).toBe("true");
    expect(bell().getAttribute("aria-controls")).toBe(panel()?.id);
    expect(container.textContent).toContain("这次打开页面以来还没有失败。");
    // The bound is a fact about the surface, so it is always stated.
    expect(container.textContent).toContain("内存中只保留最近 50 条");
    const clear = container.querySelector<HTMLButtonElement>(".notification-inbox__head button");
    expect(clear?.disabled).toBe(true);
  });

  it("badges a failure, lists it newest first, and marks it read on opening", async () => {
    await mount();
    await notify({ kind: "error", message: "第一个失败" });
    await notify({ kind: "success", message: "保存成功" });
    await notify({ kind: "error", message: "第二个失败" });

    // Only failures count, and the badge says how many are unread.
    expect(container.querySelector(".notification-inbox__badge")?.textContent).toBe("2");
    expect(bell().getAttribute("aria-label")).toBe("打开通知收件箱（2 条未读）");

    await clickBell();

    expect(rows().map((row) => row.textContent)).toEqual([
      expect.stringContaining("第二个失败"),
      expect.stringContaining("第一个失败"),
    ]);
    expect(container.textContent).not.toContain("保存成功");
    // Opening is what marked them read.
    expect(container.querySelector(".notification-inbox__badge")).toBeNull();
    expect(
      container.querySelectorAll('.notification-inbox__item[data-unread="true"]').length,
    ).toBe(0);
    const time = container.querySelector(".notification-inbox__meta time");
    expect(time?.getAttribute("dateTime")).toMatch(/^\d{4}-\d{2}-\d{2}T/);
  });

  it("keeps a failure that arrives while the list is open unread", async () => {
    await mount();
    await notify({ kind: "error", message: "先失败" });
    await clickBell();
    await notify({ kind: "error", message: "刚刚失败" });

    expect(container.querySelector(".notification-inbox__badge")?.textContent).toBe("1");
    // Two rows, and only the one that arrived after the list was opened is
    // marked: the row the reader already read stays read.
    const unread = container.querySelectorAll(
      '.notification-inbox__item[data-unread="true"]',
    );
    expect(rows().length).toBe(2);
    expect(unread.length).toBe(1);
    expect(unread[0]?.textContent).toContain("刚刚失败");
  });

  it("only makes a row clickable when there is somewhere to go", async () => {
    await mount();
    await notify({
      kind: "error",
      message: "会话「背景会话」执行失败。",
      target: { kind: "session", sessionId: "session-9" },
    });
    await notify({ kind: "error", message: "没有去向的失败" });
    await clickBell();

    const clickable = container.querySelectorAll("button.notification-inbox__row");
    expect(clickable.length).toBe(1);
    expect(clickable[0]?.textContent).toContain("打开会话");
    // The row with no target still shows the failure; it just cannot navigate.
    const staticRow = container.querySelector<HTMLElement>(
      "div.notification-inbox__row",
    );
    expect(staticRow?.textContent).toContain("没有去向的失败");
    expect(staticRow?.textContent).not.toContain("打开");

    await act(async () => {
      (clickable[0] as HTMLButtonElement).click();
    });

    expect(opened).toEqual([{ kind: "session", sessionId: "session-9" }]);
    // Navigating closes the list and hands focus back to the bell: the row that
    // was activated no longer exists.
    expect(panel()).toBeNull();
    expect(document.activeElement).toBe(bell());
  });

  it("points a provider failure at the settings section that can fix it", async () => {
    await mount();
    await notify({
      kind: "error",
      message: "「gateway」连接失败：超时",
      target: { kind: "settings", section: "providers" },
    });
    await clickBell();

    const row = container.querySelector<HTMLButtonElement>(
      "button.notification-inbox__row",
    );
    expect(row?.textContent).toContain("打开设置");

    await act(async () => {
      row?.click();
    });
    expect(opened).toEqual([{ kind: "settings", section: "providers" }]);
  });

  it("says the destination is gone instead of closing on a click that did nothing", async () => {
    unresolved = true;
    await mount();
    await notify({
      kind: "error",
      message: "会话「已删除」执行失败。",
      target: { kind: "session", sessionId: "session-gone" },
    });
    await clickBell();

    await act(async () => {
      container.querySelector<HTMLButtonElement>("button.notification-inbox__row")?.click();
    });

    expect(opened).toEqual([{ kind: "session", sessionId: "session-gone" }]);
    // The list stays where the reader is looking and explains itself there.
    expect(panel()).not.toBeNull();
    expect(container.textContent).toContain("这条通知指向的会话已不在目录中。");

    // Opening it again starts clean: the answer belonged to that click.
    await act(async () => {
      bell().click();
    });
    await act(async () => {
      bell().click();
    });
    expect(container.textContent).not.toContain("这条通知指向的会话已不在目录中。");
  });

  it("closes on Escape and hands focus back to the bell", async () => {
    await mount();
    await notify({ kind: "error", message: "失败" });
    await clickBell();
    expect(document.activeElement).toBe(panel());

    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });

    expect(panel()).toBeNull();
    expect(document.activeElement).toBe(bell());
  });

  it("closes when the reader clicks outside it", async () => {
    await mount();
    await notify({ kind: "error", message: "失败" });
    await clickBell();

    await act(async () => {
      document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    });

    expect(panel()).toBeNull();
  });

  it("clears the list without pretending the failure never happened", async () => {
    await mount();
    await notify({ kind: "error", message: "失败" });
    await clickBell();

    const clear = container.querySelector<HTMLButtonElement>(
      ".notification-inbox__head button",
    );
    expect(clear?.disabled).toBe(false);
    await act(async () => {
      clear?.click();
    });

    expect(rows()).toEqual([]);
    expect(container.textContent).toContain("这次打开页面以来还没有失败。");
    // The history is gone, so the badge cannot come back from it.
    expect(container.querySelector(".notification-inbox__badge")).toBeNull();
    // Clearing disables the button that was activated; focus stays on the panel
    // instead of falling to the document body.
    expect(document.activeElement).toBe(panel());
  });

  it("closes again when the bell is pressed a second time", async () => {
    await mount();
    await clickBell();
    expect(panel()).not.toBeNull();

    await clickBell();
    expect(panel()).toBeNull();
    expect(bell().getAttribute("aria-expanded")).toBe("false");
  });

  it("still opens, and stays inert, without a toast provider", async () => {
    // `useToast` has a documented fallback for a preview or a focused test; the
    // inbox must render and behave instead of throwing without a provider, and
    // the fallback must not invent failures or navigation.
    const bare = document.createElement("div");
    document.body.append(bare);
    const bareRoot = createRoot(bare);
    await act(async () => {
      bareRoot.render(
        <CopyProvider>
          <NotificationInbox
            onOpenTarget={() => {
              throw new Error("the fallback inbox must not navigate");
            }}
          />
        </CopyProvider>,
      );
    });
    const bareBell = bare.querySelector<HTMLButtonElement>(".notification-inbox__bell");
    expect(bareBell).not.toBeNull();
    await act(async () => {
      bareBell?.click();
    });
    expect(bare.querySelector(".notification-inbox__panel")).not.toBeNull();
    expect(bare.textContent).toContain("这次打开页面以来还没有失败。");
    expect(bare.querySelector(".notification-inbox__badge")).toBeNull();
    // Nothing to clear, so the action is not offered as if it did something.
    expect(
      bare.querySelector<HTMLButtonElement>(".notification-inbox__head button")?.disabled,
    ).toBe(true);

    await act(async () => {
      bareRoot.unmount();
    });
    bare.remove();
  });
});
