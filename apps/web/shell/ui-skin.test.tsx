import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { FALLBACK_UI_SKIN, UiSkinProvider, useUiSkin, type UiSkin } from "./ui-skin";

// The project uses a Node test environment. Capture the layout phase explicitly
// without adding a DOM dependency; server rendering still uses real React hooks.
const lifecycle = vi.hoisted(() => ({
  client: false,
  skin: "graphite" as "graphite" | "warm",
  layoutEffects: [] as Array<() => void>,
}));

vi.mock("react", async (importOriginal) => {
  const react = await importOriginal<typeof import("react")>();
  return {
    ...react,
    useState: (initial: UiSkin) => {
      const serverState = react.useState(initial);
      return lifecycle.client
        ? [lifecycle.skin, (next: UiSkin) => { lifecycle.skin = next; }]
        : serverState;
    },
    useLayoutEffect: (effect: () => void) => {
      if (lifecycle.client) lifecycle.layoutEffects.push(effect);
    },
  };
});

let selectSkin: (skin: UiSkin) => void;

function Consumer() {
  const { skin, setSkin } = useUiSkin();
  selectSkin = setSkin;
  return <div data-skin={skin}>{skin}</div>;
}

function renderProvider() {
  return renderToStaticMarkup(<UiSkinProvider><Consumer /></UiSkinProvider>);
}

function flushLayout() {
  const effects = lifecycle.layoutEffects.splice(0);
  for (const effect of effects) effect();
}

function storage(raw: string | null = null) {
  const values = new Map<string, string>();
  if (raw !== null) values.set("rove.ui-skin", raw);
  const localStorage = {
    getItem: vi.fn((key: string) => values.get(key) ?? null),
    setItem: vi.fn((key: string, value: string) => { values.set(key, value); }),
  };
  vi.stubGlobal("window", { localStorage });
  return localStorage;
}

beforeEach(() => {
  lifecycle.client = false;
  lifecycle.skin = "graphite";
  lifecycle.layoutEffects = [];
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("UiSkinProvider", () => {
  it("renders graphite on the server without browser globals", () => {
    vi.stubGlobal("window", undefined);
    expect(renderProvider()).toBe('<div data-skin="graphite">graphite</div>');
  });

  it("keeps the graphite fallback outside the provider inert", () => {
    const cache = storage("warm");
    expect(renderToStaticMarkup(<Consumer />)).toContain('data-skin="graphite"');
    selectSkin("warm");
    FALLBACK_UI_SKIN.setSkin();
    expect(FALLBACK_UI_SKIN.skin).toBe("graphite");
    expect(cache.setItem).not.toHaveBeenCalled();
  });

  it.each(["graphite", "warm"] as const)(
    "matches server markup before restoring %s in the layout phase without writing storage",
    (stored) => {
      const cache = storage(stored);
      const serverMarkup = renderProvider();
      expect(cache.getItem).not.toHaveBeenCalled();
      lifecycle.client = true;
      expect(renderProvider()).toBe(serverMarkup);
      flushLayout();
      expect(renderProvider()).toContain(`data-skin="${stored}"`);
      expect(cache.getItem).toHaveBeenCalledWith("rove.ui-skin");
      expect(cache.setItem).not.toHaveBeenCalled();
    },
  );

  it("migrates a stored cool preference to graphite once", () => {
    const cache = storage("cool");
    lifecycle.client = true;
    renderProvider();
    flushLayout();
    expect(lifecycle.skin).toBe("graphite");
    expect(cache.setItem).toHaveBeenCalledWith("rove.ui-skin", "graphite");
  });

  it.each([null, "", "dark", "GRAPHITE", " graphite "])(
    "ignores unsupported stored skin %s without rewriting it",
    (stored) => {
      const cache = storage(stored);
      lifecycle.client = true;
      renderProvider();
      flushLayout();
      expect(lifecycle.skin).toBe("graphite");
      expect(cache.setItem).not.toHaveBeenCalled();
    },
  );

  it("persists both supported choices and restores the selection on remount", () => {
    const cache = storage();
    lifecycle.client = true;
    renderProvider();
    flushLayout();
    selectSkin("warm");
    expect(lifecycle.skin).toBe("warm");
    expect(cache.setItem).toHaveBeenLastCalledWith("rove.ui-skin", "warm");
    selectSkin("graphite");
    expect(lifecycle.skin).toBe("graphite");
    expect(cache.setItem).toHaveBeenLastCalledWith("rove.ui-skin", "graphite");
    selectSkin("warm");
    lifecycle.skin = "graphite";
    renderProvider();
    flushLayout();
    expect(lifecycle.skin).toBe("warm");
  });

  it("rejects unsupported runtime choices without changing state or storage", () => {
    const cache = storage("warm");
    lifecycle.client = true;
    renderProvider();
    flushLayout();
    selectSkin("dark" as UiSkin);
    expect(lifecycle.skin).toBe("warm");
    expect(cache.setItem).not.toHaveBeenCalled();
  });

  it("keeps selection usable when accessing localStorage throws", () => {
    vi.stubGlobal("window", {
      get localStorage() { throw new Error("Storage disabled"); },
    });
    lifecycle.client = true;
    renderProvider();
    expect(flushLayout).not.toThrow();
    expect(lifecycle.skin).toBe("graphite");
    expect(() => selectSkin("warm")).not.toThrow();
    expect(lifecycle.skin).toBe("warm");
  });

  it("keeps the in-memory selection when storage is full", () => {
    const cache = storage();
    cache.setItem.mockImplementation(() => { throw new Error("Quota exceeded"); });
    lifecycle.client = true;
    renderProvider();
    flushLayout();
    expect(() => selectSkin("warm")).not.toThrow();
    expect(lifecycle.skin).toBe("warm");
  });
});
