import type { PlatformAdapter } from "../platform/types";
import { webPlatform } from "../platform/web";

export function readJsonStore<T>(
  key: string,
  fallback: T,
  platform: PlatformAdapter = webPlatform,
): T {
  const raw = platform.storageGet(key);
  if (!raw) {
    return fallback;
  }
  try {
    return JSON.parse(raw) as T;
  } catch {
    return fallback;
  }
}

export function writeJsonStore<T>(
  key: string,
  value: T,
  platform: PlatformAdapter = webPlatform,
): void {
  platform.storageSet(key, JSON.stringify(value));
}
