"use client";

import { useCallback, useEffect, useRef, useState } from "react";

import { useCopy } from "../copy/CopyProvider";
import type { ProductRuntimeInfo } from "./settings-platform-api-types";
import type { SettingsPlatformClient } from "./settings-platform-client";

type RuntimeViewState =
  | { status: "loading"; info: null; error: null }
  | { status: "refreshing"; info: ProductRuntimeInfo; error: null }
  | { status: "ready"; info: ProductRuntimeInfo; error: null }
  | { status: "error"; info: null; error: string };

export interface RuntimeSettingsProps {
  client: SettingsPlatformClient;
  connectionLabel: string;
  theme: "light" | "dark";
}

function describeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function statusLabel(value: string): string {
  return value.replaceAll("_", " ");
}

export function RuntimeSettings({
  client,
  connectionLabel,
  theme,
}: RuntimeSettingsProps) {
  const { t } = useCopy();
  const [state, setState] = useState<RuntimeViewState>({
    status: "loading",
    info: null,
    error: null,
  });
  const requestGenerationRef = useRef(0);
  const abortRef = useRef<AbortController | null>(null);

  const loadRuntimeInfo = useCallback(() => {
    const generation = requestGenerationRef.current + 1;
    requestGenerationRef.current = generation;
    abortRef.current?.abort();

    const controller = new AbortController();
    abortRef.current = controller;
    setState((current) =>
      current.info === null
        ? { status: "loading", info: null, error: null }
        : { status: "refreshing", info: current.info, error: null },
    );

    void client
      .getRuntimeInfo({ signal: controller.signal })
      .then((info) => {
        if (
          requestGenerationRef.current === generation &&
          !controller.signal.aborted
        ) {
          setState({ status: "ready", info, error: null });
        }
      })
      .catch((error: unknown) => {
        if (
          requestGenerationRef.current === generation &&
          !controller.signal.aborted
        ) {
          setState({
            status: "error",
            info: null,
            error: describeError(error),
          });
        }
      });
  }, [client]);

  useEffect(() => {
    loadRuntimeInfo();
    return () => {
      requestGenerationRef.current += 1;
      abortRef.current?.abort();
    };
  }, [loadRuntimeInfo]);

  const busy = state.status === "loading" || state.status === "refreshing";

  return (
    <section
      className="settings-panel"
      aria-labelledby="runtime-settings-title"
      aria-busy={busy}
    >
      <h1 id="runtime-settings-title">{t("settings.sectionAbout")}</h1>
      <p className="lede">
        {t("about.lede")}
      </p>

      <div className="settings-card">
        <h2>{t("settings.runtime.connection")}</h2>
        <div className="inspector-kv">
          <div>
            <span>{t("about.apiProxy")}</span>
            <strong>/api → rove-api</strong>
          </div>
          <div>
            <span>{t("settings.runtime.status")}</span>
            <strong>{connectionLabel}</strong>
          </div>
          <div>
            <span>{t("about.host")}</span>
            <strong>web</strong>
          </div>
          <div>
            <span>{t("settings.runtime.theme")}</span>
            <strong>{theme === "dark" ? t("settings.general.themeDark") : t("settings.general.themeLight")}</strong>
          </div>
          {state.info ? (
            <>
              <div>
                <span>{t("about.apiVersion")}</span>
                <strong>{state.info.api_version}</strong>
              </div>
              <div>
                <span>{t("about.runtimeConnection")}</span>
                <strong>{state.info.connection}</strong>
              </div>
            </>
          ) : null}
        </div>
        {state.status === "ready" || state.status === "refreshing" ? (
          <div className="field-actions">
            <button
              type="button"
              className="secondary"
              onClick={loadRuntimeInfo}
              disabled={busy}
            >
              {state.status === "refreshing"
                ? t("about.refreshing")
                : t("about.refresh")}
            </button>
          </div>
        ) : null}
      </div>

      {state.status === "loading" ? (
        <div className="placeholder-note" role="status" aria-live="polite">
          {t("about.loading")}
        </div>
      ) : null}

      {state.status === "error" ? (
        <div className="settings-card">
          <h2>{t("about.unavailableTitle")}</h2>
          <div className="shell-alert" role="alert">
            {state.error}
          </div>
          <div className="field-actions">
            <button type="button" onClick={loadRuntimeInfo}>
              {t("common.retry")}
            </button>
          </div>
        </div>
      ) : null}

      {state.info ? (
        <RuntimeHealthCards info={state.info} refreshing={state.status === "refreshing"} />
      ) : null}
    </section>
  );
}

function RuntimeHealthCards({
  info,
  refreshing,
}: {
  info: ProductRuntimeInfo;
  refreshing: boolean;
}) {
  const { t } = useCopy();
  return (
    <>
      <div className="settings-card">
        <h2>{t("settings.runtime.dataStorage")}</h2>
        <div className="inspector-kv">
          <div>
            <span>{t("settings.runtime.status")}</span>
            <strong>{statusLabel(info.product_store)}</strong>
          </div>
        </div>
        {info.product_store === "unavailable" ? (
          <div className="placeholder-note" role="status">
            {t("about.storeUnavailable")}
          </div>
        ) : null}
      </div>

      <div className="settings-card">
        <h2>{t("settings.runtime.runtime")}</h2>
        <div className="inspector-kv">
          <div>
            <span>{t("about.adapter")}</span>
            <strong>{statusLabel(info.execution_environment.adapter)}</strong>
          </div>
          <div>
            <span>{t("about.workspaceKind")}</span>
            <strong>{statusLabel(info.execution_environment.workspace_kind)}</strong>
          </div>
          <div>
            <span>{t("about.workspaceIdentity")}</span>
            <strong className="settings-digest">
              {info.execution_environment.workspace_digest}
            </strong>
          </div>
          {Object.entries(info.execution_environment.capabilities).map(
            ([capability, enabled]) => (
              <div key={capability}>
                <span>{capabilityLabel(capability, t)}</span>
                <strong>
                  {enabled ? t("about.available") : t("about.unavailable")}
                </strong>
              </div>
            ),
          )}
        </div>
      </div>

      <div className="settings-card">
        <h2>{t("about.selector")}</h2>
        <div className="inspector-kv">
          <div>
            <span>{t("about.selector")}</span>
            <strong>{info.agent.selector}</strong>
          </div>
          <div>
            <span>{t("about.workspaceSource")}</span>
            <strong>
              {info.agent.workspace_source_authorized
                ? t("about.authorized")
                : t("about.restricted")}
            </strong>
          </div>
          <div>
            <span>{t("about.workspaceInstructions")}</span>
            <strong>
              {info.agent.workspace_instructions_enabled
                ? t("about.enabled")
                : t("about.disabled")}
            </strong>
          </div>
          <div>
            <span>{t("about.remediationProcedures")}</span>
            <strong>
              {info.agent.allow_remediation_procedures
                ? t("about.allowed")
                : t("about.diagnosticOnly")}
            </strong>
          </div>
          <div>
            <span>{t("about.procedureLimit")}</span>
            <strong>{info.agent.max_procedure_selections}</strong>
          </div>
        </div>
      </div>

      {info.resume_health ? (
        <div className="settings-card" aria-live="polite">
          <h2>{t("settings.runtime.sessionRecovery")}</h2>
          <div className="inspector-kv">
            <div>
              <span>{t("settings.runtime.status")}</span>
              <strong>{statusLabel(info.resume_health.status)}</strong>
            </div>
            <div>
              <span>{t("about.resumeWorkspaces")}</span>
              <strong>{info.resume_health.workspace_count}</strong>
            </div>
            <div>
              <span>{t("about.resumeSessions")}</span>
              <strong>{info.resume_health.session_count}</strong>
            </div>
            <div>
              <span>{t("about.resumeBound")}</span>
              <strong>{info.resume_health.bound_session_count}</strong>
            </div>
            <div>
              <span>{t("about.resumeRunning")}</span>
              <strong>{info.resume_health.running_session_count}</strong>
            </div>
            <div>
              <span>{t("about.resumeAttention")}</span>
              <strong>{info.resume_health.needs_attention_session_count}</strong>
            </div>
          </div>
          {refreshing ? <span role="status">{t("about.refreshing")}</span> : null}
        </div>
      ) : null}
    </>
  );
}

function capabilityLabel(
  capability: string,
  t: (path: string) => string,
): string {
  switch (capability) {
    case "filesystem_read":
      return t("about.filesystemRead");
    case "filesystem_write":
      return t("about.filesystemWrite");
    case "process_run":
      return t("about.processRun");
    case "process_stdio":
      return t("about.processStdio");
    case "process_background":
      return t("about.processBackground");
    case "process_pty":
      return t("about.processPty");
    case "observations":
      return t("about.observations");
    default:
      return statusLabel(capability);
  }
}
