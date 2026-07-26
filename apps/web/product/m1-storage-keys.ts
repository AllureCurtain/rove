/** Exact browser-owned M1 inputs accepted by the one-time C0 migration. */
export const M1_BROWSER_SOURCE_SCHEMA_VERSION = 1 as const;

export const M1_BROWSER_STORAGE_KEYS = {
  workspaces: "rove.product.workspaces",
  sessions: "rove.product.sessions",
  active: "rove.product.active",
  providerProfiles: "rove.product.providerProfiles",
  providerSelection: "rove.product.providerSelection",
  theme: "rove.theme",
} as const;

/**
 * Durable client-side pending/complete receipt state. The migration worker must
 * never delete or enumerate legacy keys and may mark this complete only after
 * validating a successful server receipt.
 */
export const M1_BROWSER_MIGRATION_STATE_KEY =
  "rove.product.migration.web-m1.v1";
