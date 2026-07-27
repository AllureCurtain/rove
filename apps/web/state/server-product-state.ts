import { webPlatform } from "../platform/web";
import type {
  ProductPreferences,
  ProductSession,
  ProductWorkspace,
  UpdateProductPreferencesRequest,
} from "../product/product-api-types";
import type { ProductApiClient } from "../product/product-client";
import type { ProductCatalog } from "./product-catalog";
import { defaultProviderSelection } from "./provider-store";
import type { ActiveProviderSelection } from "./product-types";
import type { SessionRecord } from "./product-types";
import { fromProductSession, fromProductWorkspace } from "./product-types";

export function selectionFromPreferences(
  preferences: ProductPreferences,
): ActiveProviderSelection {
  const saved = preferences.provider_selection;
  if (!saved) {
    return defaultProviderSelection();
  }
  return {
    mode: saved.profile_id ? "profile" : "default",
    profileId: saved.profile_id,
    model: saved.model,
    approval: saved.approval,
    maxSteps: saved.max_steps,
  };
}

export function toPreferencesRequest(
  preferences: ProductPreferences,
): UpdateProductPreferencesRequest {
  return {
    schema_version: preferences.schema_version,
    theme: preferences.theme,
    active_workspace_id: preferences.active_workspace_id,
    active_session_id: preferences.active_session_id,
    provider_selection: preferences.provider_selection,
  };
}

export function resolveProductTheme(
  preference: ProductPreferences["theme"],
): "light" | "dark" {
  return preference === "system"
    ? webPlatform.resolveTheme("system")
    : preference;
}

export async function listSessionsBounded(
  client: ProductApiClient,
  workspaceIds: string[],
): Promise<ProductSession[]> {
  const sessions: ProductSession[] = [];
  const concurrency = 6;
  for (let index = 0; index < workspaceIds.length; index += concurrency) {
    const batch = workspaceIds.slice(index, index + concurrency);
    const responses = await Promise.all(
      batch.map((workspaceId) => client.listSessions(workspaceId)),
    );
    for (const response of responses) {
      sessions.push(...response.sessions);
    }
  }
  return sessions;
}

export function mergeWorkspaceSnapshot(
  catalog: ProductCatalog,
  workspace: ProductWorkspace,
  sessions: ProductSession[],
): ProductCatalog {
  const workspaceRecord = fromProductWorkspace(workspace);
  const sessionRecords = sessions
    .filter((session) => session.status !== "archived")
    .map(fromProductSession);
  return {
    ...catalog,
    workspaces: [
      workspaceRecord,
      ...catalog.workspaces.filter((item) => item.id !== workspaceRecord.id),
    ],
    sessions: [
      ...sessionRecords,
      ...catalog.sessions.filter(
        (session) => session.workspaceId !== workspaceRecord.id,
      ),
    ],
  };
}

export function hasAdvancedRuntimeBinding(
  before: SessionRecord,
  after: SessionRecord | null,
): after is SessionRecord & { activeJobId: string } {
  if (!after?.activeJobId) {
    return false;
  }
  const beforeOrdinal = before.runtimeOrdinal ?? 0;
  const afterOrdinal = after.runtimeOrdinal ?? 0;
  return (
    afterOrdinal > beforeOrdinal ||
    (afterOrdinal === beforeOrdinal && after.activeJobId !== before.activeJobId)
  );
}
