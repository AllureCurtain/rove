"use client";

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  createTranslator,
  getDictionary,
  type CopyDictionary,
  type Locale,
} from "./index";
import {
  DEFAULT_LOCALE,
  isLocale,
  readStoredLocale,
  writeStoredLocale,
} from "./locales";

type CopyContextValue = {
  locale: Locale;
  setLocale: (locale: Locale) => void;
  dict: CopyDictionary;
  t: (
    path: string,
    params?: Record<string, string | number>,
  ) => string;
};

const CopyContext = createContext<CopyContextValue | null>(null);

export function CopyProvider({ children }: { children: ReactNode }) {
  const [locale, setLocaleState] = useState<Locale>(DEFAULT_LOCALE);
  // The layout bootstrap already painted the stored locale onto <html>; the
  // first mount effect must not clobber it back to the build default.
  const restoredRef = useRef(false);

  useEffect(() => {
    const stored = readStoredLocale();
    if (stored && stored !== locale) {
      setLocaleState(stored);
    }
    restoredRef.current = true;
    if (typeof document !== "undefined") {
      document.documentElement.lang = stored ?? DEFAULT_LOCALE;
    }
    // Read once on mount; subsequent changes go through setLocale.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!restoredRef.current || typeof document === "undefined") {
      return;
    }
    document.documentElement.lang = locale;
  }, [locale]);

  const setLocale = useCallback((next: Locale) => {
    if (!isLocale(next)) {
      return;
    }
    setLocaleState(next);
    writeStoredLocale(next);
  }, []);

  const value = useMemo<CopyContextValue>(() => {
    const dict = getDictionary(locale);
    return {
      locale,
      setLocale,
      dict,
      t: createTranslator(locale),
    };
  }, [locale, setLocale]);

  return <CopyContext.Provider value={value}>{children}</CopyContext.Provider>;
}

const FallbackCopy: CopyContextValue = {
  locale: DEFAULT_LOCALE,
  setLocale: () => undefined,
  dict: getDictionary(DEFAULT_LOCALE),
  t: createTranslator(DEFAULT_LOCALE),
};

export function useCopy(): CopyContextValue {
  return useContext(CopyContext) ?? FallbackCopy;
}
