import { createTranslator } from "../copy";
import type { ProviderProfileInput } from "../state/product-types";

const FAILURE_KEYS: Record<string, string> = {
  provider_timeout: "timeout",
  provider_authentication: "authentication",
  provider_rate_limited: "rateLimited",
  provider_protocol_mismatch: "protocol",
  provider_no_models: "model",
  provider_model_unavailable: "model",
  product_revision_conflict: "conflict",
  provider_catalog_conflict: "conflict",
  provider_credential_store: "credentialStore",
  provider_reconciliation_required: "reconciliation",
  provider_product_projection: "reconciliation",
  provider_transport: "transport",
  provider_upstream: "upstream",
  provider_onboarding_invalid: "invalid",
  product_provider_invalid: "invalid",
  product_invalid_input: "invalid",
  bad_request: "inputOrCredential",
  provider_onboarding_loopback_required: "localOnly",
  project_trust_required: "trust",
};

export function describeProviderProbeFailure(
  error: unknown,
  t: (key: string) => string = createTranslator("en-US"),
): string {
  const code = typeof error === "object" && error !== null && "code" in error
    && typeof error.code === "string" ? error.code : "";
  return t(`settings.providers.errors.${FAILURE_KEYS[code] ?? "unknown"}`);
}

export function providerDraftIssue(
  input: ProviderProfileInput,
  requireModel = true,
): { field: "base" | "default-model" | "key-env"; copyKey: string } | null {
  if (input.providerType !== "fake") {
    try {
      const url = new URL(input.apiBase);
      if (!["http:", "https:"].includes(url.protocol) || url.username || url.password || url.search || url.hash) {
        throw new Error("invalid endpoint");
      }
    } catch {
      return { field: "base", copyKey: "settings.providers.errors.address" };
    }
  }
  if (requireModel && !input.defaultModel?.trim()) {
    return { field: "default-model", copyKey: "settings.providers.errors.modelRequired" };
  }
  if (input.apiKeyEnv && (input.apiKeyEnv.length > 256 || !/^[A-Z_][A-Z0-9_]*$/u.test(input.apiKeyEnv))) {
    return { field: "key-env", copyKey: "settings.providers.errors.envName" };
  }
  return null;
}
