"use client";

import { useEffect, useMemo, useRef, useState } from "react";

import { useCopy } from "../copy/CopyProvider";
import { createProductApiClient } from "../product/product-client";
import type {
  ProductAuthorizationRecord,
  ProductAuthorizationsResponse,
} from "../product/product-api-types";

/**
 * One durable authorization request + decision record projected from the
 * product authorizations endpoint (plan P4, decision side).
 *
 * Fields the durable store never recorded (`decided_via` on pre-schema-v5
 * rows, outcomes for decisions that never reached execution) stay unknown
 * instead of being reconstructed by guessing.
 */
export type SessionAuthorizationRecord = ProductAuthorizationRecord;

type SessionAuthorizationState =
  | { status: "loading"; sessionId: string }
  | { status: "error"; sessionId: string; error: string }
  | {
      status: "ready";
      sessionId: string;
      records: SessionAuthorizationRecord[];
      truncated: boolean;
    };

/** Bound the section; the endpoint already pages newest-first. */
const RECORD_LIMIT = 50;

function shortId(value: string): string {
  return value.length <= 12 ? value : `${value.slice(0, 8)}…${value.slice(-4)}`;
}

/**
 * Normalize one bounded authorizations page for rendering.
 *
 * The endpoint already deduplicates by durable `call_id` and orders newest
 * update first. This projection only bounds the page and never invents a
 * decision, actor, or time that the store did not record.
 */
export function collectSessionAuthorizationRecords(
  response: ProductAuthorizationsResponse,
): SessionAuthorizationRecord[] {
  return response.authorizations.slice(0, RECORD_LIMIT);
}

function decisionLabel(
  record: SessionAuthorizationRecord,
  t: (key: string) => string,
): string {
  switch (record.status) {
    case "pending":
      return t("inspector.authorizationStatusPending");
    case "approved":
      return t("inspector.authorizationStatusApproved");
    case "rejected":
      return t("inspector.authorizationStatusRejected");
    case "cancelled":
      return t("inspector.authorizationStatusCancelled");
    case "interrupted":
      return t("inspector.authorizationStatusInterrupted");
    default:
      return record.status;
  }
}

function actorLabel(
  record: SessionAuthorizationRecord,
  t: (key: string) => string,
): string {
  if (!record.decided_via) {
    return t("inspector.authorizationUnknown");
  }
  switch (record.decided_via) {
    case "job_api":
      return t("inspector.authorizationActorJobApi");
    case "job_cancel":
      return t("inspector.authorizationActorJobCancel");
    case "job_responder_lost":
      return t("inspector.authorizationActorResponderLost");
    default:
      return record.decided_via;
  }
}

function outcomeLabel(
  record: SessionAuthorizationRecord,
  t: (key: string) => string,
): string {
  if (!record.outcome) {
    return t("inspector.authorizationUnknown");
  }
  return record.outcome.event === "tool_call_failed"
    ? t("inspector.authorizationOutcomeFailed")
    : t("inspector.authorizationOutcomeCompleted");
}

export function SessionAuthorizationPanel({
  sessionId,
  workspaceId,
}: {
  sessionId: string;
  workspaceId: string;
}) {
  const { t } = useCopy();
  const client = useMemo(() => createProductApiClient(), []);
  const [state, setState] = useState<SessionAuthorizationState>({
    status: "loading",
    sessionId,
  });
  const generationRef = useRef(0);

  useEffect(() => {
    const generation = ++generationRef.current;
    setState({ status: "loading", sessionId });
    void (async () => {
      try {
        const response = await client.getSessionAuthorizations(sessionId, {
          limit: RECORD_LIMIT,
        });
        // A late response for a session the user already left must not publish.
        if (generationRef.current !== generation) {
          return;
        }
        if (response.session_id !== sessionId) {
          throw new Error(
            "Authorizations response does not match the session route.",
          );
        }
        setState({
          status: "ready",
          sessionId,
          records: collectSessionAuthorizationRecords(response),
          truncated: response.truncated,
        });
      } catch (caught) {
        if (generationRef.current !== generation) {
          return;
        }
        setState({
          status: "error",
          sessionId,
          error:
            caught instanceof Error
              ? caught.message
              : "Failed to load authorization records",
        });
      }
    })();
    return () => {
      generationRef.current += 1;
    };
    // workspaceId is kept in the signature for route identity checks at the
    // call site; the authorizations endpoint derives its workspace from the
    // durable session binding instead of trusting a client-supplied id.
  }, [client, sessionId]);

  // A ready result for a session the user already left must never be rendered
  // as this session's history.
  const ready = useMemo(
    () =>
      state.status === "ready" && state.sessionId === sessionId ? state : null,
    [state, sessionId],
  );
  const records = ready?.records ?? [];

  return (
    <section
      className="authorization-history"
      aria-label={t("inspector.authorizationTitle")}
      data-workspace-id={workspaceId}
    >
      <h3>{t("inspector.authorizationTitle")}</h3>
      <p className="authorization-history__limit" role="note">
        {t("inspector.authorizationHistoryNote")}
      </p>

      {state.status === "loading" ? (
        <p role="status">{t("inspector.authorizationLoading")}</p>
      ) : null}

      {state.status === "error" ? (
        <p role="alert">{t("inspector.authorizationError")}</p>
      ) : null}

      {ready?.truncated ? (
        <p role="status">{t("inspector.authorizationPartial")}</p>
      ) : null}

      {ready && records.length === 0 ? (
        <p role="status">{t("inspector.authorizationNone")}</p>
      ) : null}

      {records.length > 0 ? (
        <table className="authorization-history__table">
          <thead>
            <tr>
              <th scope="col">{t("inspector.authorizationColumnTool")}</th>
              <th scope="col">{t("inspector.authorizationColumnCall")}</th>
              <th scope="col">{t("inspector.authorizationColumnScope")}</th>
              <th scope="col">{t("inspector.authorizationColumnDecision")}</th>
              <th scope="col">{t("inspector.authorizationColumnActor")}</th>
              <th scope="col">{t("inspector.authorizationColumnTime")}</th>
              <th scope="col">{t("inspector.authorizationColumnOutcome")}</th>
            </tr>
          </thead>
          <tbody>
            {records.map((record) => (
              <tr key={record.call_id} data-status={record.status}>
                <td>
                  <strong>{record.tool}</strong>
                  <small title={record.reason}>{record.reason}</small>
                </td>
                <td>
                  <code title={record.call_id}>{shortId(record.call_id)}</code>
                </td>
                <td>
                  <span title={record.run_id}>
                    {t("inspector.authorizationRunLabel")}
                  </span>
                </td>
                <td>{decisionLabel(record, t)}</td>
                <td>{actorLabel(record, t)}</td>
                <td>{record.updated_at}</td>
                <td>{outcomeLabel(record, t)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}
    </section>
  );
}
