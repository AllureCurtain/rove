import { invoke } from "@tauri-apps/api/core";

import { desktopTransport } from "./desktop-transport";

type DesktopInvoke = <T>(command: string) => Promise<T>;

export function desktopWorkspacePickerAvailable(): boolean {
  return desktopTransport() !== null;
}

export async function selectDesktopWorkspace(
  invokeImpl: DesktopInvoke = invoke,
): Promise<string | null> {
  if (!desktopWorkspacePickerAvailable()) {
    return null;
  }
  const selected = await invokeImpl<unknown>("workspace_select");
  if (selected === null) {
    return null;
  }
  if (typeof selected !== "string" || selected.trim() === "") {
    throw new Error("Desktop returned an invalid workspace path.");
  }
  return selected;
}
