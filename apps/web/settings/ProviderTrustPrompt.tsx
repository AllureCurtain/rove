"use client";

import { CheckIcon, Cross2Icon } from "@radix-ui/react-icons";
import { useCallback, useEffect, useState } from "react";

import { useCopy } from "../copy/CopyProvider";
import type { SettingsPlatformClient } from "./settings-platform-client";

/**
 * The workspace trust prompt for provider credentials.
 *
 * The product turn path refuses to run a non-fake provider profile until the
 * workspace holds the `provider_credentials` Project Trust capability
 * (`apps/api/src/assembly.rs`), and the failure lands in the message queue as
 * an abandoned successor with only `project_trust_required` for a reason.
 * Nothing else in the UI says so, so this card is the one place that names the
 * missing grant next to the panel that configured the provider — and offers
 * the one click that grants it. The grant is an explicit decision, matching
 * the trust model: it is never made silently.
 *
 * It renders only while the grant is actually missing — a workspace that
 * already holds the capability, a session on the local demo provider, and a
 * shell with no active workspace all get nothing.
 */
export function ProviderTrustPrompt({
  client,
  workspaceId,
  providerIsRemote,
  workspaceName,
}: {
  client: SettingsPlatformClient;
  /** The workspace the session would run in; nothing renders without it. */
  workspaceId: string | null;
  /**
   * Whether the selected provider is a real one — the gate only applies to
   * non-fake profiles, so the prompt is offered exactly where the failure
   * would be.
   */
  providerIsRemote: boolean;
  /** The workspace's display name, so the prompt can name what is granted. */
  workspaceName?: string;
}) {
  const { t } = useCopy();
  // `null` = still loading or unavailable; the prompt only exists once the
  // server's answer says the capability is missing.
  const [missingGrant, setMissingGrant] = useState<boolean | null>(null);
  const [granting, setGranting] = useState(false);
  const [granted, setGranted] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dismissed, setDismissed] = useState(false);

  const load = useCallback(
    async (signal?: AbortSignal) => {
      if (!workspaceId || !providerIsRemote) {
        setMissingGrant(null);
        return;
      }
      try {
        const status = await client.getProjectTrust(workspaceId, { signal });
        setMissingGrant(
          !status.granted_capabilities.includes("provider_credentials"),
        );
      } catch {
        // The trust read failing must not block the panel: the prompt is
        // absent rather than wrong, and the actual failure still surfaces the
        // next time a turn is attempted.
        if (!signal?.aborted) {
          setMissingGrant(null);
        }
      }
    },
    [client, workspaceId, providerIsRemote],
  );

  useEffect(() => {
    setGranted(false);
    setDismissed(false);
    setError(null);
    const controller = new AbortController();
    void load(controller.signal);
    return () => controller.abort();
  }, [load]);

  async function grant() {
    if (!workspaceId || granting) {
      return;
    }
    setGranting(true);
    setError(null);
    try {
      const status = await client.decideProjectTrust(workspaceId, {
        decision: "grant",
        capabilities: ["provider_credentials"],
      });
      if (status.granted_capabilities.includes("provider_credentials")) {
        setMissingGrant(false);
        setGranted(true);
      } else {
        setMissingGrant(
          !status.granted_capabilities.includes("provider_credentials"),
        );
      }
    } catch (grantError) {
      setError(
        grantError instanceof Error ? grantError.message : String(grantError),
      );
    } finally {
      setGranting(false);
    }
  }

  if (!workspaceId || !providerIsRemote || dismissed) {
    return null;
  }
  if (granted) {
    return (
      <div className="settings-card" role="status">
        <p className="settings-inline-note">
          {t("settings.providers.trustGranted", {
            workspace: workspaceName ?? workspaceId,
          })}
        </p>
      </div>
    );
  }
  if (missingGrant !== true) {
    return null;
  }
  return (
    <div className="settings-card" role="alert">
      <div className="settings-card__heading">
        <div>
          <h2>{t("settings.providers.trustTitle")}</h2>
          <p className="settings-inline-note">
            {t("settings.providers.trustRequired", {
              workspace: workspaceName ?? workspaceId,
            })}
          </p>
        </div>
        <button
          type="button"
          className="secondary"
          aria-label={t("settings.providers.trustDismiss")}
          title={t("settings.providers.trustDismiss")}
          onClick={() => setDismissed(true)}
        >
          <Cross2Icon />
        </button>
      </div>
      <div className="field-actions">
        <button type="button" disabled={granting} onClick={() => void grant()}>
          <CheckIcon />{" "}
          {granting
            ? t("settings.providers.trustGrantBusy")
            : t("settings.providers.trustGrant")}
        </button>
      </div>
      {error ? (
        <p className="chat-error" role="alert">
          {t("settings.providers.trustGrantFailed", { reason: error })}
        </p>
      ) : null}
    </div>
  );
}
