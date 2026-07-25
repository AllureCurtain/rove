import type { ProviderType } from "../lib/rove-types";
import type { PlatformAdapter } from "../platform/types";
import { webPlatform } from "../platform/web";
import { readJsonStore, writeJsonStore } from "./local-store";
import {
  newId,
  type ActiveProviderSelection,
  type ProviderProfileRecord,
} from "./product-types";

const PROFILES_KEY = "rove.product.providerProfiles";
const SELECTION_KEY = "rove.product.providerSelection";

export function loadProviderProfiles(
  platform: PlatformAdapter = webPlatform,
): ProviderProfileRecord[] {
  return readJsonStore<ProviderProfileRecord[]>(PROFILES_KEY, [], platform);
}

export function saveProviderProfiles(
  profiles: ProviderProfileRecord[],
  platform: PlatformAdapter = webPlatform,
): void {
  writeJsonStore(PROFILES_KEY, profiles, platform);
}

export function defaultProviderSelection(): ActiveProviderSelection {
  return {
    mode: "default",
    model: "fake",
    approval: "ask",
    maxSteps: 8,
  };
}

export function loadProviderSelection(
  platform: PlatformAdapter = webPlatform,
): ActiveProviderSelection {
  return readJsonStore(
    SELECTION_KEY,
    defaultProviderSelection(),
    platform,
  );
}

export function saveProviderSelection(
  selection: ActiveProviderSelection,
  platform: PlatformAdapter = webPlatform,
): void {
  writeJsonStore(SELECTION_KEY, selection, platform);
}

export function upsertProviderProfile(
  profiles: ProviderProfileRecord[],
  input: {
    id?: string;
    label: string;
    providerType: ProviderType;
    apiBase: string;
    apiKeyEnv?: string;
    defaultModel?: string;
  },
): ProviderProfileRecord[] {
  const now = new Date().toISOString();
  if (input.id) {
    const exists = profiles.some((profile) => profile.id === input.id);
    if (exists) {
      return profiles.map((profile) =>
        profile.id === input.id
          ? {
              ...profile,
              label: input.label,
              providerType: input.providerType,
              apiBase: input.apiBase,
              apiKeyEnv: input.apiKeyEnv,
              defaultModel: input.defaultModel,
              updatedAt: now,
            }
          : profile,
      );
    }
  }

  const record: ProviderProfileRecord = {
    id: input.id ?? newId("prov"),
    label: input.label,
    providerType: input.providerType,
    apiBase: input.apiBase,
    apiKeyEnv: input.apiKeyEnv,
    defaultModel: input.defaultModel,
    updatedAt: now,
  };
  return [record, ...profiles.filter((profile) => profile.id !== record.id)];
}

export function removeProviderProfile(
  profiles: ProviderProfileRecord[],
  profileId: string,
): ProviderProfileRecord[] {
  return profiles.filter((profile) => profile.id !== profileId);
}

export function providerRequiresKey(type: ProviderType): boolean {
  return type === "openai" || type === "openai-responses" || type === "anthropic";
}

export function providerDefaultApiBase(type: ProviderType): string {
  switch (type) {
    case "anthropic":
      return "https://api.anthropic.com";
    case "ollama":
      return "http://localhost:11434";
    case "fake":
      return "local";
    case "openai":
    case "openai-responses":
      return "https://api.openai.com/v1";
  }
}

export function providerDefaultKeyEnv(type: ProviderType): string {
  switch (type) {
    case "anthropic":
      return "ANTHROPIC_API_KEY";
    case "openai":
    case "openai-responses":
      return "OPENAI_API_KEY";
    default:
      return "";
  }
}

export function providerDisplayName(type: ProviderType | "default"): string {
  switch (type) {
    case "openai":
      return "OpenAI";
    case "openai-responses":
      return "OpenAI Responses";
    case "anthropic":
      return "Anthropic";
    case "ollama":
      return "Ollama";
    case "fake":
      return "Fake";
    default:
      return "Runtime default";
  }
}
