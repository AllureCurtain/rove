import type { ProductPreferences } from "../product/product-api-types";
import { SETTINGS_SECTIONS, type SettingsSectionId } from "../settings/sections";
import type { ProductCatalog } from "./product-catalog";

export type ProductRoute =
  | { kind: "root" }
  | { kind: "workspace"; workspaceId: string }
  | { kind: "session"; workspaceId: string; sessionId: string }
  | { kind: "settings"; section: SettingsSectionId | null }
  | { kind: "invalid" };

const SETTINGS_IDS = new Set<string>(SETTINGS_SECTIONS.map((section) => section.id));

export function parseProductRoute(pathname: string): ProductRoute {
  const segments = pathname.split("/").filter(Boolean);
  if (segments.length === 0) {
    return { kind: "root" };
  }
  if (segments[0] === "settings") {
    if (segments.length === 1) {
      return { kind: "settings", section: null };
    }
    if (segments.length === 2 && SETTINGS_IDS.has(segments[1]!)) {
      return { kind: "settings", section: segments[1] as SettingsSectionId };
    }
    return { kind: "invalid" };
  }
  if (segments[0] !== "w" || (segments.length !== 2 && segments.length !== 4)) {
    return { kind: "invalid" };
  }
  const workspaceId = decodeRouteSegment(segments[1]!);
  if (!workspaceId) {
    return { kind: "invalid" };
  }
  if (segments.length === 2) {
    return { kind: "workspace", workspaceId };
  }
  if (segments[2] !== "s") {
    return { kind: "invalid" };
  }
  const sessionId = decodeRouteSegment(segments[3]!);
  return sessionId
    ? { kind: "session", workspaceId, sessionId }
    : { kind: "invalid" };
}

export function sessionHref(workspaceId: string, sessionId: string): string {
  return `/w/${encodeURIComponent(workspaceId)}/s/${encodeURIComponent(sessionId)}`;
}

export function workspaceHref(workspaceId: string): string {
  return `/w/${encodeURIComponent(workspaceId)}`;
}

export function settingsHref(section: SettingsSectionId): string {
  return `/settings/${section}`;
}

export function preferredProductHref(
  catalog: ProductCatalog,
  preferences: ProductPreferences,
): string | null {
  const preferredSession = catalog.sessions.find(
    (session) =>
      session.id === preferences.active_session_id &&
      session.workspaceId === preferences.active_workspace_id,
  );
  if (preferredSession) {
    return sessionHref(preferredSession.workspaceId, preferredSession.id);
  }
  const firstSession = catalog.sessions[0];
  if (firstSession) {
    return sessionHref(firstSession.workspaceId, firstSession.id);
  }
  const preferredWorkspace = catalog.workspaces.find(
    (workspace) => workspace.id === preferences.active_workspace_id,
  );
  const workspace = preferredWorkspace ?? catalog.workspaces[0];
  return workspace ? workspaceHref(workspace.id) : null;
}

export function defaultWorkspaceHref(
  catalog: ProductCatalog,
  workspaceId: string,
  preferredSessionId?: string,
): string | null {
  const sessions = catalog.sessions.filter(
    (session) => session.workspaceId === workspaceId,
  );
  const preferred = sessions.find((session) => session.id === preferredSessionId);
  const session = preferred ?? sessions[0];
  return session ? sessionHref(workspaceId, session.id) : null;
}

function decodeRouteSegment(value: string): string | null {
  try {
    return decodeURIComponent(value) || null;
  } catch {
    return null;
  }
}
