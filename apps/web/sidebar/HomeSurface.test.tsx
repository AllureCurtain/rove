// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CopyProvider } from "../copy/CopyProvider";
import type {
  SessionRecord,
  WorkspaceKind,
  WorkspaceRecord,
} from "../state/product-types";
import { HomeSurface } from "./HomeSurface";

/**
 * The §9.1 home surface: hero + shared composer slot + inline workspace
 * switcher + recents + a demoted manual path entry. Mounted in jsdom because
 * the contract is interactive — the switcher menu, the shared dialog, and the
 * blocked-send escalation only exist across user events.
 */

const WORKSPACES: WorkspaceRecord[] = [
  {
    id: "ws-a",
    rootPath: "D:/code/alpha",
    kind: "repo",
    displayName: "alpha",
    pinned: false,
    lastOpenedAt: "2026-10-01T08:00:00Z",
  },
  {
    id: "ws-b",
    rootPath: "D:/code/beta",
    kind: "folder",
    displayName: "beta",
    pinned: false,
    lastOpenedAt: "2026-10-02T08:00:00Z",
  },
];

const SESSIONS: SessionRecord[] = [
  {
    id: "session-9",
    workspaceId: "ws-a",
    title: "Reconcile the ledger",
    createdAt: "2026-10-03T08:00:00Z",
    updatedAt: "2026-10-04T09:30:00Z",
    status: "idle",
    archived: false,
    hasDurableTurn: true,
    lastOutcome: "success",
    lastOutcomeAt: "2026-10-04T09:30:00Z",
  },
];

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT =
    true;
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

function renderSurface(options?: {
  workspace?: WorkspaceRecord | null;
  workspaces?: WorkspaceRecord[];
  recentSessions?: SessionRecord[];
  sendBlocked?: {
    reason: string;
    action: "workspace" | "providers";
    alerted: boolean;
  } | null;
  onSelectWorkspace?: (workspaceId: string) => void;
  onSelectSession?: (workspaceId: string, sessionId: string) => void;
  onOpenWorkspace?: (path: string, kind: WorkspaceKind) => void;
  onOpenProviders?: () => void;
}) {
  return (
    <CopyProvider>
      <HomeSurface
        workspace={options?.workspace ?? null}
        workspaces={options?.workspaces ?? []}
        recentSessions={options?.recentSessions ?? []}
        workspaceName={(id) => WORKSPACES.find((w) => w.id === id)?.displayName}
        composer={<div data-testid="home-composer" />}
        sendBlocked={options?.sendBlocked ?? null}
        onSelectWorkspace={options?.onSelectWorkspace ?? (() => {})}
        onSelectSession={options?.onSelectSession ?? (() => {})}
        onOpenWorkspace={options?.onOpenWorkspace ?? (() => {})}
        onOpenProviders={options?.onOpenProviders ?? (() => {})}
      />
    </CopyProvider>
  );
}

async function flush() {
  await act(async () => {});
}

function click(element: Element | null) {
  expect(element).not.toBeNull();
  act(() => {
    element!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

describe("HomeSurface", () => {
  it("renders the hero, the composer slot, and the demoted manual entry", async () => {
    act(() => {
      root.render(renderSurface());
    });
    await flush();

    expect(container.querySelector(".home-surface__mark")).not.toBeNull();
    expect(container.querySelector("h1")?.textContent).toBe("让 rove 做点什么？");
    expect(container.querySelector("[data-testid='home-composer']")).not.toBeNull();

    // The absolute-path form exists but stays inside the collapsed details.
    const details = container.querySelector("details.home-surface__manual");
    expect(details).not.toBeNull();
    expect(details!.hasAttribute("open")).toBe(false);
    expect(details!.querySelector("input")).not.toBeNull();
  });

  it("opens the shared dialog straight from the switcher when no workspace is known", async () => {
    act(() => {
      root.render(renderSurface());
    });
    await flush();

    click(container.querySelector(".home-surface__switcher-button"));
    await flush();

    const dialog = container.querySelector("[role='dialog']");
    expect(dialog).not.toBeNull();
    expect(dialog!.querySelector("h2")?.textContent).toBe("打开工作区");
  });

  it("lists workspaces in the switcher menu and selects one", async () => {
    const onSelectWorkspace = vi.fn();
    act(() => {
      root.render(
        renderSurface({
          workspace: WORKSPACES[1],
          workspaces: WORKSPACES,
          onSelectWorkspace,
        }),
      );
    });
    await flush();

    const switcher = container.querySelector<HTMLButtonElement>(
      ".home-surface__switcher-button",
    )!;
    expect(switcher.textContent).toContain("beta");

    click(switcher);
    await flush();

    const menu = container.querySelector("[role='menu']");
    expect(menu).not.toBeNull();
    const items = Array.from(menu!.querySelectorAll("button"));
    const alpha = items.find((item) => item.textContent === "alpha")!;
    const beta = items.find((item) => item.textContent === "beta")!;
    expect(beta.getAttribute("aria-checked")).toBe("true");

    click(alpha);
    await flush();
    expect(onSelectWorkspace).toHaveBeenCalledWith("ws-a");
    expect(container.querySelector("[role='menu']")).toBeNull();
  });

  it("keeps the open-workspace entry inside the switcher menu", async () => {
    act(() => {
      root.render(
        renderSurface({ workspace: WORKSPACES[0], workspaces: WORKSPACES }),
      );
    });
    await flush();

    click(container.querySelector(".home-surface__switcher-button"));
    await flush();
    const openItem = Array.from(
      container.querySelectorAll("[role='menu'] button"),
    ).find((item) => item.textContent === "打开文件夹或仓库…");
    click(openItem ?? null);
    await flush();

    expect(container.querySelector("[role='dialog']")).not.toBeNull();
    expect(container.querySelector("[role='menu']")).toBeNull();
  });

  it("renders recents with workspace name and forwards the selection", async () => {
    const onSelectSession = vi.fn();
    act(() => {
      root.render(
        renderSurface({ recentSessions: SESSIONS, onSelectSession }),
      );
    });
    await flush();

    const recent = container.querySelector(".home-surface__recent");
    expect(recent).not.toBeNull();
    expect(recent!.textContent).toContain("Reconcile the ledger");
    expect(recent!.textContent).toContain("alpha");
    expect(recent!.textContent).toContain("2026-10-04");

    click(recent);
    await flush();
    expect(onSelectSession).toHaveBeenCalledWith("ws-a", "session-9");
  });

  it("submits the manual path entry through the shared open flow", async () => {
    const onOpenWorkspace = vi.fn();
    act(() => {
      root.render(renderSurface({ onOpenWorkspace }));
    });
    await flush();

    const form = container.querySelector<HTMLFormElement>(
      ".home-surface__manual form",
    )!;
    const input = form.querySelector<HTMLInputElement>("input")!;
    const select = form.querySelector<HTMLSelectElement>("select")!;
    act(() => {
      const setter = Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        "value",
      )!.set!;
      setter.call(input, "  D:/code/gamma  ");
      input.dispatchEvent(new Event("input", { bubbles: true }));
      select.value = "repo";
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await flush();
    act(() => {
      form.dispatchEvent(
        new Event("submit", { bubbles: true, cancelable: true }),
      );
    });
    await flush();

    expect(onOpenWorkspace).toHaveBeenCalledWith("D:/code/gamma", "repo");
  });

  it("shows the blocked-send hint and routes the providers action", async () => {
    const onOpenProviders = vi.fn();
    act(() => {
      root.render(
        renderSurface({
          sendBlocked: {
            reason: "发送前请先配置模型服务。",
            action: "providers",
            alerted: false,
          },
          onOpenProviders,
        }),
      );
    });
    await flush();

    const hint = container.querySelector(".home-surface__send-hint");
    expect(hint).not.toBeNull();
    expect(hint!.getAttribute("role")).toBe("status");
    expect(hint!.textContent).toContain("发送前请先配置模型服务。");

    click(hint!.querySelector("button"));
    await flush();
    expect(onOpenProviders).toHaveBeenCalledTimes(1);
  });

  it("escalates the hint to an alert and opens the dialog for the workspace action", async () => {
    act(() => {
      root.render(
        renderSurface({
          sendBlocked: {
            reason: "请先打开工作区再发送。",
            action: "workspace",
            alerted: true,
          },
        }),
      );
    });
    await flush();

    const hint = container.querySelector(".home-surface__send-hint");
    expect(hint!.getAttribute("role")).toBe("alert");

    click(hint!.querySelector("button"));
    await flush();
    expect(container.querySelector("[role='dialog']")).not.toBeNull();
  });
});
