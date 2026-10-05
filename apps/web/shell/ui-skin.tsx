"use client";

import {
  createContext,
  useCallback,
  useContext,
  useLayoutEffect,
  useState,
  type ReactNode,
} from "react";

export type UiSkin = "graphite" | "warm";

const UI_SKIN_STORAGE_KEY = "rove.ui-skin";
const DEFAULT_UI_SKIN: UiSkin = "graphite";

const UiSkinContext = createContext<{
  skin: UiSkin;
  setSkin: (skin: UiSkin) => void;
}>({ skin: DEFAULT_UI_SKIN, setSkin: () => undefined });

function readStoredUiSkin(): UiSkin | null {
  if (typeof window === "undefined") {
    return null;
  }
  try {
    const raw = window.localStorage.getItem(UI_SKIN_STORAGE_KEY);
    if (raw === "warm" || raw === "graphite") {
      return raw;
    }
    // The retired "cool" study folded into graphite; migrate once so the next
    // visit does not re-hit this branch.
    if (raw === "cool") {
      writeStoredUiSkin("graphite");
      return "graphite";
    }
    return null;
  } catch {
    return null;
  }
}

function writeStoredUiSkin(skin: UiSkin): void {
  if (typeof window === "undefined") {
    return;
  }
  try {
    window.localStorage.setItem(UI_SKIN_STORAGE_KEY, skin);
  } catch {
    // Skin preference is optional.
  }
}

export function UiSkinProvider({ children }: { children: ReactNode }) {
  const [skin, setSkinState] = useState<UiSkin>(DEFAULT_UI_SKIN);

  // Keep the server and first client render on the default skin, then restore
  // before paint.
  useLayoutEffect(() => {
    const stored = readStoredUiSkin();
    if (stored) {
      setSkinState(stored);
    }
  }, []);

  const setSkin = useCallback((next: UiSkin) => {
    if (next !== "warm" && next !== "graphite") {
      return;
    }
    setSkinState(next);
    writeStoredUiSkin(next);
  }, []);

  return (
    <UiSkinContext.Provider value={{ skin, setSkin }}>
      {children}
    </UiSkinContext.Provider>
  );
}

export function useUiSkin() {
  return useContext(UiSkinContext);
}

/** Safe default when a component mounts outside UiSkinProvider (e.g. storybook/dev tools). */
export const FALLBACK_UI_SKIN = {
  skin: DEFAULT_UI_SKIN as UiSkin,
  setSkin: () => undefined,
};
