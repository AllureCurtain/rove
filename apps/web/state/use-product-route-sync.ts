"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { usePathname, useRouter } from "next/navigation";

import type { ProductPreferences } from "../product/product-api-types";
import type { SettingsSectionId } from "../settings/sections";
import { findSession, findWorkspace, type ProductCatalog } from "./product-catalog";
import {
  defaultWorkspaceHref,
  parseProductRoute,
  preferredProductHref,
  sessionHref,
  settingsHref,
  workspaceHref,
} from "./product-route";

export function useProductRouteSync({
  ready,
  catalog,
  preferences,
  selectCatalogRoute,
  persistActiveRoute,
  focusSession,
  prepareSession,
  leaveSession,
}: {
  ready: boolean;
  catalog: ProductCatalog;
  preferences: ProductPreferences | null;
  selectCatalogRoute: (workspaceId: string | null, sessionId: string | null) => void;
  persistActiveRoute: (workspaceId: string, sessionId: string | undefined) => void;
  focusSession: (workspaceId: string, sessionId: string) => void;
  prepareSession: (sessionId: string) => void;
  leaveSession: () => void;
}) {
  const router = useRouter();
  const pathname = usePathname();
  const navigationGenerationRef = useRef(0);
  const route = useMemo(() => parseProductRoute(pathname), [pathname]);
  const [routeFailure, setRouteFailure] = useState<{
    pathname: string;
    error: string;
  } | null>(null);
  const routeError =
    routeFailure?.pathname === pathname ? routeFailure.error : null;
  const pushRoute = useCallback(
    (href: string) => {
      ++navigationGenerationRef.current;
      router.push(href);
    },
    [router],
  );
  const replaceRoute = useCallback(
    (href: string) => {
      ++navigationGenerationRef.current;
      router.replace(href);
    },
    [router],
  );
  const captureNavigationIntent = useCallback(
    () => navigationGenerationRef.current,
    [],
  );
  const isNavigationIntentCurrent = useCallback(
    (generation: number) => navigationGenerationRef.current === generation,
    [],
  );

  useEffect(() => {
    const recordBrowserNavigation = () => {
      ++navigationGenerationRef.current;
    };
    window.addEventListener("popstate", recordBrowserNavigation);
    return () => window.removeEventListener("popstate", recordBrowserNavigation);
  }, []);
  const routePending = useMemo(() => {
    if (!ready || !preferences || routeError) {
      return false;
    }
    if (route.kind === "root") {
      return preferredProductHref(catalog, preferences) !== null;
    }
    if (route.kind === "settings") {
      return route.section === null;
    }
    if (route.kind === "invalid") {
      return true;
    }
    const workspace = findWorkspace(catalog, route.workspaceId);
    if (!workspace) {
      return true;
    }
    if (route.kind === "workspace") {
      if (
        defaultWorkspaceHref(
          catalog,
          workspace.id,
          preferences.active_session_id,
        )
      ) {
        return true;
      }
      return (
        catalog.active.workspaceId !== workspace.id ||
        catalog.active.sessionId !== null
      );
    }
    const session = findSession(catalog, route.sessionId);
    if (!session || session.workspaceId !== workspace.id) {
      return true;
    }
    return (
      catalog.active.workspaceId !== workspace.id ||
      catalog.active.sessionId !== session.id
    );
  }, [catalog, preferences, ready, route, routeError]);

  useEffect(() => {
    if (!ready || !preferences) {
      return;
    }
    if (route.kind === "root") {
      setRouteFailure(null);
      const href = preferredProductHref(catalog, preferences);
      if (href) {
        replaceRoute(href);
      } else {
        leaveSession();
        if (
          catalog.active.workspaceId !== null ||
          catalog.active.sessionId !== null
        ) {
          selectCatalogRoute(null, null);
        }
      }
      return;
    }
    if (route.kind === "settings") {
      setRouteFailure(null);
      if (!route.section) {
        replaceRoute(settingsHref("general"));
      } else {
        leaveSession();
      }
      return;
    }
    if (route.kind === "invalid") {
      leaveSession();
      setRouteFailure({
        pathname,
        error: "This product route is not recognized.",
      });
      return;
    }

    const workspace = findWorkspace(catalog, route.workspaceId);
    if (!workspace) {
      leaveSession();
      setRouteFailure({
        pathname,
        error: "The requested workspace is not present in the server catalog.",
      });
      return;
    }
    if (route.kind === "workspace") {
      setRouteFailure(null);
      const href = defaultWorkspaceHref(
        catalog,
        workspace.id,
        preferences.active_session_id,
      );
      if (href) {
        replaceRoute(href);
      } else {
        leaveSession();
        if (
          catalog.active.workspaceId !== workspace.id ||
          catalog.active.sessionId !== null
        ) {
          selectCatalogRoute(workspace.id, null);
        }
        persistActiveRoute(workspace.id, undefined);
      }
      return;
    }

    const session = findSession(catalog, route.sessionId);
    if (!session || session.workspaceId !== workspace.id) {
      leaveSession();
      setRouteFailure({
        pathname,
        error: "The requested session does not belong to this workspace.",
      });
      return;
    }
    setRouteFailure(null);
    if (
      catalog.active.workspaceId !== workspace.id ||
      catalog.active.sessionId !== session.id
    ) {
      selectCatalogRoute(workspace.id, session.id);
    }
    persistActiveRoute(workspace.id, session.id);
    focusSession(workspace.id, session.id);
  }, [
    catalog,
    focusSession,
    leaveSession,
    persistActiveRoute,
    preferences,
    pathname,
    ready,
    replaceRoute,
    route,
    selectCatalogRoute,
  ]);

  const navigateSession = useCallback(
    (workspaceId: string, sessionId: string) => {
      prepareSession(sessionId);
      pushRoute(sessionHref(workspaceId, sessionId));
    },
    [prepareSession, pushRoute],
  );

  const navigateWorkspace = useCallback(
    (workspaceId: string) => {
      leaveSession();
      pushRoute(workspaceHref(workspaceId));
    },
    [leaveSession, pushRoute],
  );

  const openSettings = useCallback(
    (section: SettingsSectionId) => pushRoute(settingsHref(section)),
    [pushRoute],
  );

  const backToChat = useCallback(() => {
    const workspace = findWorkspace(catalog, catalog.active.workspaceId);
    const session = findSession(catalog, catalog.active.sessionId);
    pushRoute(
      workspace && session
        ? sessionHref(workspace.id, session.id)
        : preferences
          ? preferredProductHref(catalog, preferences) ?? "/"
          : "/",
    );
  }, [catalog, preferences, pushRoute]);

  return {
    route,
    routeError,
    routePending,
    settingsSection:
      route.kind === "settings" && route.section ? route.section : "general",
    viewSettings: route.kind === "settings",
    navigateSession,
    navigateWorkspace,
    openSettings,
    backToChat,
    returnHome: () => replaceRoute("/"),
    captureNavigationIntent,
    isNavigationIntentCurrent,
  };
}
