"use client";

import { useCallback, useEffect, useRef, useState } from "react";

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
      <h1 id="runtime-settings-title">About / Runtime</h1>
      <p className="lede">
        Live API, product persistence, and exact-resume health.
      </p>

      <div className="settings-card">
        <h2>Connection</h2>
        <div className="inspector-kv">
          <div>
            <span>API proxy</span>
            <strong>/api → rove-api</strong>
          </div>
          <div>
            <span>status</span>
            <strong>{connectionLabel}</strong>
          </div>
          <div>
            <span>host</span>
            <strong>web</strong>
          </div>
          <div>
            <span>theme</span>
            <strong>{theme}</strong>
          </div>
          {state.info ? (
            <>
              <div>
                <span>API version</span>
                <strong>{state.info.api_version}</strong>
              </div>
              <div>
                <span>runtime connection</span>
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
                ? "Refreshing…"
                : "Refresh runtime"}
            </button>
          </div>
        ) : null}
      </div>

      {state.status === "loading" ? (
        <div className="placeholder-note" role="status" aria-live="polite">
          Loading runtime health…
        </div>
      ) : null}

      {state.status === "error" ? (
        <div className="settings-card">
          <h2>Runtime information unavailable</h2>
          <div className="shell-alert" role="alert">
            {state.error}
          </div>
          <div className="field-actions">
            <button type="button" onClick={loadRuntimeInfo}>
              Retry
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
  return (
    <>
      <div className="settings-card">
        <h2>ProductStore</h2>
        <div className="inspector-kv">
          <div>
            <span>status</span>
            <strong>{statusLabel(info.product_store)}</strong>
          </div>
        </div>
        {info.product_store === "unavailable" ? (
          <div className="placeholder-note" role="status">
            ProductStore is unavailable. Durable workspace, session, and resume
            health cannot be read.
          </div>
        ) : null}
      </div>

      {info.resume_health ? (
        <div className="settings-card" aria-live="polite">
          <h2>Resume health</h2>
          <div className="inspector-kv">
            <div>
              <span>status</span>
              <strong>{statusLabel(info.resume_health.status)}</strong>
            </div>
            <div>
              <span>workspaces</span>
              <strong>{info.resume_health.workspace_count}</strong>
            </div>
            <div>
              <span>sessions</span>
              <strong>{info.resume_health.session_count}</strong>
            </div>
            <div>
              <span>bound sessions</span>
              <strong>{info.resume_health.bound_session_count}</strong>
            </div>
            <div>
              <span>running sessions</span>
              <strong>{info.resume_health.running_session_count}</strong>
            </div>
            <div>
              <span>sessions needing attention</span>
              <strong>{info.resume_health.needs_attention_session_count}</strong>
            </div>
          </div>
          {refreshing ? <span role="status">Refreshing runtime health…</span> : null}
        </div>
      ) : null}
    </>
  );
}
