import { webPlatform } from "../platform/web";
import { MAX_PRODUCT_SESSION_PAGE_LIMIT } from "../product/product-api-types";
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
    return {
      ...defaultProviderSelection(),
      approval: preferences.default_approval_policy,
    };
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
    expected_revision: preferences.revision,
    theme: preferences.theme,
    default_approval_policy: preferences.default_approval_policy,
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

/**
 * Pages the API can serve for one workspace before we stop asking.
 *
 * At the maximum page size this covers more sessions than a workspace can hold,
 * so reaching it means a cursor stopped advancing. Stopping there turns that
 * into a short list rather than a request loop that never ends.
 */
const MAX_SESSION_PAGES_PER_WORKSPACE = 64;

async function listWorkspaceSessions(
  client: ProductApiClient,
  workspaceId: string,
): Promise<ProductSession[]> {
  const sessions: ProductSession[] = [];
  let cursor: string | undefined;
  for (let page = 0; page < MAX_SESSION_PAGES_PER_WORKSPACE; page += 1) {
    // Archived sessions are dropped by every consumer of this catalog, so we
    // ask the server not to send them rather than paying to transfer and
    // discard them.
    const response = await client.listSessions(workspaceId, {
      cursor,
      limit: MAX_PRODUCT_SESSION_PAGE_LIMIT,
      includeArchived: false,
    });
    sessions.push(...response.sessions);
    if (!response.next_cursor) {
      return sessions;
    }
    cursor = response.next_cursor;
  }
  return sessions;
}

export async function listSessionsBounded(
  client: ProductApiClient,
  workspaceIds: string[],
): Promise<ProductSession[]> {
  const sessions: ProductSession[] = [];
  const concurrency = 6;
  for (let index = 0; index < workspaceIds.length; index += concurrency) {
    const batch = workspaceIds.slice(index, index + concurrency);
    const perWorkspace = await Promise.all(
      batch.map((workspaceId) => listWorkspaceSessions(client, workspaceId)),
    );
    for (const workspaceSessions of perWorkspace) {
      sessions.push(...workspaceSessions);
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
): after is SessionRecord & { activeJobId: string; activeRunId: string } {
  if (!after?.activeJobId || !after.activeRunId) {
    return false;
  }
  const beforeOrdinal = before.runtimeOrdinal ?? 0;
  const afterOrdinal = after.runtimeOrdinal ?? 0;
  return (
    afterOrdinal > beforeOrdinal ||
    (afterOrdinal === beforeOrdinal && after.activeJobId !== before.activeJobId)
  );
}
