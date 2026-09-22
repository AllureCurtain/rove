"use client";

import { useEffect, useMemo, useRef, useState } from "react";

import { useCopy } from "../copy/CopyProvider";
import { createProductApiClient } from "../product/product-client";
import type {
  ProductTranscriptResponse,
  ProductTranscriptStatus,
} from "../product/product-api-types";

/**
 * One durable authorization request record projected from a product session.
 *
 * Only the request side is represented. The durable decision side does not
 * exist yet, so nothing here claims who decided, when, or with what outcome.
 */
export interface SessionAuthorizationRecord {
  callId: string;
  toolName: string;
  runId: string;
  runOrdinal: number;
  seq: number;
  reason: string;
  /** True when the run was inherited by this session rather than started here. */
  inherited: boolean;
}

type SessionAuthorizationState =
  | { status: "loading"; sessionId: string }
  | { status: "error"; sessionId: string; error: string }
  | {
      status: "ready";
      sessionId: string;
      records: SessionAuthorizationRecord[];
      transcriptStatus: ProductTranscriptStatus;
    };

/** Bound the section; the transcript itself is already bounded server-side. */
const RECORD_LIMIT = 50;

/**
 * Collect the durable request side of every tool authorization in a session.
 *
 * Only `tool_call_approval_needed` canonical events are read. No canonical
 * decision event exists, so the decision, its actor and its time stay unknown
 * rather than being inferred from a later `tool_call_completed` /
 * `tool_call_failed` event: a completed tool does not prove who authorized it
 * (plan P4).
 *
 * Records are keyed by call id so a replayed event cannot report one request
 * twice, and ordered newest-first by canonical sequence.
 */
export function collectSessionAuthorizationRecords(
  transcript: ProductTranscriptResponse,
): SessionAuthorizationRecord[] {
  const byCall = new Map<string, SessionAuthorizationRecord>();
  for (const segment of transcript.segments) {
    const binding = segment.binding;
    for (const event of segment.events) {
      const detail = event.event;
      if (detail.type !== "tool_call_approval_needed") {
        continue;
      }
      if (byCall.has(detail.call_id)) {
        continue;
      }
      byCall.set(detail.call_id, {
        callId: detail.call_id,
        toolName: detail.name,
        runId: binding.runtime_run_id,
        runOrdinal: binding.ordinal,
        seq: event.seq,
        reason: detail.reason,
        inherited: segment.inherited,
      });
    }
  }
  return [...byCall.values()]
    .sort((left, right) => right.seq - left.seq)
    .slice(0, RECORD_LIMIT);
}

function shortId(value: string): string {
  return value.length <= 12 ? value : `${value.slice(0, 8)}…${value.slice(-4)}`;
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
        const transcript = await client.getTranscript(sessionId);
        // A late response for a session the user already left must not publish.
        if (generationRef.current !== generation) {
          return;
        }
        if (transcript.workspace_id !== workspaceId) {
          throw new Error("Transcript workspace does not match the session route.");
        }
        setState({
          status: "ready",
          sessionId,
          records: collectSessionAuthorizationRecords(transcript),
          transcriptStatus: transcript.status,
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
  }, [client, sessionId, workspaceId]);

  // A ready result for a session the user already left must never be rendered
  // as this session's history.
  const records = useMemo(
    () =>
      state.status === "ready" && state.sessionId === sessionId
        ? state.records
        : [],
    [state, sessionId],
  );

  return (
    <section
      className="authorization-history"
      aria-label={t("inspector.authorizationTitle")}
    >
      <h3>{t("inspector.authorizationTitle")}</h3>
      <p className="authorization-history__limit" role="note">
        {t("inspector.authorizationHistoryUnavailable")}
      </p>

      {state.status === "loading" ? (
        <p role="status">{t("inspector.authorizationLoading")}</p>
      ) : null}

      {state.status === "error" ? (
        <p role="alert">{t("inspector.authorizationError")}</p>
      ) : null}

      {state.status === "ready" && state.transcriptStatus !== "complete" ? (
        <p role="status">{t("inspector.authorizationPartial")}</p>
      ) : null}

      {state.status === "ready" && records.length === 0 ? (
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
            </tr>
          </thead>
          <tbody>
            {records.map((record) => (
              <tr key={record.callId}>
                <td>
                  <strong>{record.toolName}</strong>
                  <small title={record.reason}>{record.reason}</small>
                </td>
                <td>
                  <code title={record.callId}>{shortId(record.callId)}</code>
                </td>
                <td>
                  <span title={record.runId}>
                    {t("inspector.authorizationRun", { ordinal: record.runOrdinal })}
                  </span>
                  {record.inherited ? (
                    <small>{t("inspector.authorizationInherited")}</small>
                  ) : null}
                </td>
                <td>{t("inspector.authorizationUnknown")}</td>
                <td>{t("inspector.authorizationUnknown")}</td>
                <td>{t("inspector.authorizationUnknown")}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}
    </section>
  );
}
