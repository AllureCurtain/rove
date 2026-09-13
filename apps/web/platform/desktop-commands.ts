import { invoke } from "@tauri-apps/api/core";

import type { ProviderType } from "../lib/rove-types";
import { desktopTransport } from "./desktop-transport";

type DesktopInvoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
type DesktopRemoteProviderType = Extract<
  ProviderType,
  "openai" | "openai-responses" | "anthropic"
>;

const PROFILE_ID_PATTERN = /^[A-Za-z0-9_.-]{1,128}$/u;
const CONTROL_CHARACTER_PATTERN = /[\u0000-\u001f\u007f]/u;
const REMOTE_PROVIDER_TYPES = new Set<ProviderType>([
  "openai",
  "openai-responses",
  "anthropic",
]);

export interface DesktopProviderCredentialRequest {
  profileId?: string;
  label: string;
  providerType: DesktopRemoteProviderType;
  apiBase: string;
  model: string;
  makeDefault?: boolean;
  expectedRevision?: string;
}

export interface DesktopProviderProbe {
  inventoryCount: number;
  streamingSupported: boolean;
  nativeToolCallsSupported: boolean;
  usageSupported: boolean;
}

export interface DesktopProviderOnboardingReceipt {
  profileId: string;
  label: string;
  providerType: DesktopRemoteProviderType;
  apiBase: string;
  model: string;
  catalogRevision: string;
  credentialSource: "keyring";
  probe: DesktopProviderProbe;
  selected: boolean;
}

export interface DesktopProviderProbeRequest {
  profileId: string;
  model?: string;
}

export interface DesktopProviderUseRequest extends DesktopProviderProbeRequest {
  expectedRevision?: string;
}

export interface DesktopProviderSelectionReceipt {
  profileId: string;
  model: string;
  catalogRevision: string;
}

function record(value: unknown, message: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(message);
  }
  return value as Record<string, unknown>;
}

function boundedText(value: string, maximum: number, message: string): string {
  const trimmed = value.trim();
  if (
    !trimmed ||
    trimmed.length > maximum ||
    CONTROL_CHARACTER_PATTERN.test(value)
  ) {
    throw new Error(message);
  }
  return trimmed;
}

function profileId(value: string): string {
  const trimmed = value.trim();
  if (!PROFILE_ID_PATTERN.test(trimmed) || trimmed !== value) {
    throw new Error("Provider profile id is invalid.");
  }
  return trimmed;
}

function optionalProfileId(value: string | undefined): string | undefined {
  return value === undefined ? undefined : profileId(value);
}

function optionalRevision(value: string | undefined): string | undefined {
  return value === undefined
    ? undefined
    : boundedText(value, 128, "Provider catalog revision is invalid.");
}

function normalizeApiBase(value: string): string {
  const normalized = boundedText(value, 2_048, "Provider API base is invalid.")
    .replace(/\/+$/u, "");
  let parsed: URL;
  try {
    parsed = new URL(normalized);
  } catch {
    throw new Error("Provider API base is invalid.");
  }
  if (!["http:", "https:"].includes(parsed.protocol) || !parsed.hostname) {
    throw new Error("Provider API base is invalid.");
  }
  return normalized;
}

function remoteProviderType(value: ProviderType): DesktopRemoteProviderType {
  if (!REMOTE_PROVIDER_TYPES.has(value)) {
    throw new Error("Native credential onboarding requires a remote Provider type.");
  }
  return value as DesktopRemoteProviderType;
}

function parseProbe(value: unknown): DesktopProviderProbe {
  const probe = record(value, "Desktop returned an invalid Provider probe.");
  if (
    typeof probe.inventory_count !== "number" ||
    !Number.isSafeInteger(probe.inventory_count) ||
    probe.inventory_count < 0 ||
    probe.inventory_count > 4_096 ||
    typeof probe.streaming_supported !== "boolean" ||
    typeof probe.native_tool_calls_supported !== "boolean" ||
    typeof probe.usage_supported !== "boolean"
  ) {
    throw new Error("Desktop returned an invalid Provider probe.");
  }
  return {
    inventoryCount: probe.inventory_count,
    streamingSupported: probe.streaming_supported,
    nativeToolCallsSupported: probe.native_tool_calls_supported,
    usageSupported: probe.usage_supported,
  };
}

function requireDesktop(): void {
  if (!desktopProviderCredentialPromptAvailable()) {
    throw new Error("Secure provider onboarding requires the Rove Desktop host.");
  }
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
): Promise<DesktopProviderOnboardingReceipt> {
  requireDesktop();
  const normalized = {
    profileId: optionalProfileId(request.profileId),
    label: boundedText(request.label, 256, "Provider profile label is invalid."),
    providerType: remoteProviderType(request.providerType),
    apiBase: normalizeApiBase(request.apiBase),
    model: boundedText(request.model, 1_024, "Provider model is invalid."),
    makeDefault: request.makeDefault ?? true,
    expectedRevision: optionalRevision(request.expectedRevision),
  };
  const value = await invokeImpl<unknown>("provider_credential_prompt", {
    request: {
      profile_id: normalized.profileId,
      label: normalized.label,
      provider_type: normalized.providerType,
      api_base: normalized.apiBase,
      model: normalized.model,
      make_default: normalized.makeDefault,
      expected_revision: normalized.expectedRevision,
    },
  });
  const receipt = record(value, "Desktop returned an invalid onboarding receipt.");
  const returnedProfileId =
    typeof receipt.profile_id === "string" ? profileId(receipt.profile_id) : null;
  if (
    returnedProfileId === null ||
    (normalized.profileId !== undefined && returnedProfileId !== normalized.profileId) ||
    receipt.label !== normalized.label ||
    receipt.provider_type !== normalized.providerType ||
    receipt.api_base !== normalized.apiBase ||
    receipt.model !== normalized.model ||
    typeof receipt.catalog_revision !== "string" ||
    optionalRevision(receipt.catalog_revision) === undefined ||
    receipt.credential_source !== "keyring" ||
    typeof receipt.selected !== "boolean"
  ) {
    throw new Error("Desktop returned an invalid onboarding receipt.");
  }
  return {
    profileId: returnedProfileId,
    label: normalized.label,
    providerType: normalized.providerType,
    apiBase: normalized.apiBase,
    model: normalized.model,
    catalogRevision: receipt.catalog_revision,
    credentialSource: "keyring",
    probe: parseProbe(receipt.probe),
    selected: receipt.selected,
  };
}

export async function probeDesktopProvider(
  request: DesktopProviderProbeRequest,
  invokeImpl: DesktopInvoke = invoke,
): Promise<DesktopProviderProbe> {
  requireDesktop();
  const normalizedProfileId = profileId(request.profileId);
  const model = request.model === undefined
    ? undefined
    : boundedText(request.model, 1_024, "Provider model is invalid.");
  const value = await invokeImpl<unknown>("provider_profile_probe", {
    request: { profile_id: normalizedProfileId, model },
  });
  return parseProbe(value);
}

export async function useDesktopProvider(
  request: DesktopProviderUseRequest,
  invokeImpl: DesktopInvoke = invoke,
): Promise<DesktopProviderSelectionReceipt> {
  requireDesktop();
  const normalizedProfileId = profileId(request.profileId);
  const model = request.model === undefined
    ? undefined
    : boundedText(request.model, 1_024, "Provider model is invalid.");
  const expectedRevision = optionalRevision(request.expectedRevision);
  const value = await invokeImpl<unknown>("provider_profile_use", {
    request: {
      profile_id: normalizedProfileId,
      model,
      expected_revision: expectedRevision,
    },
  });
  const receipt = record(value, "Desktop returned an invalid Provider selection receipt.");
  if (
    receipt.profile_id !== normalizedProfileId ||
    typeof receipt.model !== "string" ||
    boundedText(
      receipt.model,
      1_024,
      "Desktop returned an invalid Provider selection receipt.",
    ) !== (model ?? receipt.model) ||
    typeof receipt.catalog_revision !== "string" ||
    optionalRevision(receipt.catalog_revision) === undefined
  ) {
    throw new Error("Desktop returned an invalid Provider selection receipt.");
  }
  return {
    profileId: normalizedProfileId,
    model: receipt.model,
    catalogRevision: receipt.catalog_revision,
  };
}
