interface CodedError {
  code: string;
  message?: string;
}

function codedError(error: unknown): CodedError | null {
  if (typeof error !== "object" || error === null || !("code" in error)) {
    return null;
  }
  const code = error.code;
  if (typeof code !== "string") {
    return null;
  }
  const message = "message" in error && typeof error.message === "string"
    ? error.message
    : undefined;
  return { code, message };
}

export function describeProviderProbeFailure(error: unknown): string {
  switch (codedError(error)?.code) {
    case "provider_timeout":
      return "Provider timed out. Check the endpoint and try again.";
    case "provider_authentication":
      return "Provider authentication failed. Re-enter or check the configured credential.";
    case "provider_rate_limited":
      return "Provider rate limit reached. Retry after the provider window resets.";
    case "provider_protocol_mismatch":
      return "Endpoint did not return a compatible model catalog.";
    case "provider_no_models":
    case "provider_model_unavailable":
      return "Provider returned no usable models.";
    case "product_revision_conflict":
      return "Provider settings changed. Reload the Catalog and retry.";
    case "provider_credential_store":
      return "Windows credential storage is unavailable.";
    case "provider_reconciliation_required":
    case "provider_product_projection":
      return "Provider setup needs reconciliation. Reload Settings before retrying.";
    case "provider_transport":
      return "Provider could not be reached. Check the endpoint and local network.";
    default:
      return error instanceof Error ? error.message : String(error);
  }
}
