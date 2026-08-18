import { invoke } from "@tauri-apps/api/core";

import { desktopTransport } from "./desktop-transport";

type DesktopInvoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

export interface DesktopProviderCredentialRequest {
  profileId: string;
  label: string;
}

export interface DesktopProviderCredentialReceipt {
  profileId: string;
  source: "keyring";
  service: string;
  account: string;
}

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

export function desktopProviderCredentialPromptAvailable(): boolean {
  return desktopTransport() !== null;
}

export async function promptDesktopProviderCredential(
  request: DesktopProviderCredentialRequest,
  invokeImpl: DesktopInvoke = invoke,
): Promise<DesktopProviderCredentialReceipt> {
  if (!desktopProviderCredentialPromptAvailable()) {
    throw new Error("Secure provider onboarding requires the Rove Desktop host.");
  }
  const profileId = request.profileId.trim();
  const label = request.label.trim();
  if (!/^[A-Za-z0-9_.-]{1,128}$/u.test(profileId) || !label || label.length > 256) {
    throw new Error("Provider credential metadata is invalid.");
  }
  const value = await invokeImpl<unknown>("provider_credential_prompt", {
    request: { profile_id: profileId, label },
  });
  if (typeof value !== "object" || value === null) {
    throw new Error("Desktop returned an invalid credential receipt.");
  }
  const receipt = value as Record<string, unknown>;
  const accountPrefix = `profile:${profileId}:`;
  if (
    receipt.profile_id !== profileId ||
    receipt.source !== "keyring" ||
    receipt.service !== "com.rove.agent.provider" ||
    typeof receipt.account !== "string" ||
    !receipt.account.startsWith(accountPrefix) ||
    !/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/iu.test(
      receipt.account.slice(accountPrefix.length),
    )
  ) {
    throw new Error("Desktop returned an invalid credential receipt.");
  }
  return {
    profileId,
    source: "keyring",
    service: receipt.service,
    account: receipt.account,
  };
}
