"use client";

import {
  createContext,
  useCallback,
  useContext,
  useLayoutEffect,
  useState,
  type ReactNode,
} from "react";

export type UiSkin = "cool" | "warm";

const UI_SKIN_STORAGE_KEY = "rove.ui-skin";

const UiSkinContext = createContext<{
  skin: UiSkin;
  setSkin: (skin: UiSkin) => void;
}>({ skin: "warm", setSkin: () => undefined });

function readStoredUiSkin(): UiSkin | null {
  if (typeof window === "undefined") {
    return null;
  }
  try {
    const raw = window.localStorage.getItem(UI_SKIN_STORAGE_KEY);
    return raw === "warm" || raw === "cool" ? raw : null;
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
  const [skin, setSkinState] = useState<UiSkin>("warm");

  // Keep the server and first client render warm, then restore before paint.
  useLayoutEffect(() => {
    const stored = readStoredUiSkin();
    if (stored) {
      setSkinState(stored);
    }
  }, []);

  const setSkin = useCallback((next: UiSkin) => {
    if (next !== "warm" && next !== "cool") {
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
  skin: "warm" as UiSkin,
  setSkin: () => undefined,
};
