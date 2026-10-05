"use client";

import { useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import { useCopy } from "../copy/CopyProvider";
import { formatUtcTimestamp } from "../lib/format-timestamp";
import type { SessionRecord } from "../state/product-types";
import {
  SESSION_HOVER_CARD_WIDTH_PX,
  placeSessionHoverCard,
  type SessionHoverCardAnchor,
} from "./session-hover-card";
import { useSessionModelSummary } from "./session-model-summary";
import { formatDisplayPath, sessionStatusLabel, shortId } from "./session-labels";

export interface SessionHoverCardProps {
  id: string;
  session: SessionRecord;
  /** Absolute root of the owning workspace, shown as the last row. */
  workspaceRootPath: string;
  /** Parent session title when the part of the tree is loaded. */
  parentTitle?: string | null;
  anchor: SessionHoverCardAnchor | null;
  /** Portal target inside the shell; `document.body` when it is missing. */
  host: HTMLElement | null;
}

/**
 * Informational hover card for one sidebar session row (design F4).
 *
 * Read-only by design: the row itself already owns the actionable affordances,
 * and a second entry point inside a hover surface would only duplicate them.
 * The model row is the one piece of data the row does not carry, so it is read
 * lazily through `useSessionModelSummary` while the card is mounted.
 */
export function SessionHoverCard({
  id,
  session,
  workspaceRootPath,
  parentTitle,
  anchor,
  host,
}: SessionHoverCardProps) {
  const { t } = useCopy();
  const summary = useSessionModelSummary(session.id);
  const cardRef = useRef<HTMLDivElement>(null);
  const [placement, setPlacement] = useState<{ top: number; left: number } | null>(null);

  useLayoutEffect(() => {
    if (!anchor) {
      return;
    }
    const measure = () => {
      const height = cardRef.current?.getBoundingClientRect().height ?? 0;
      setPlacement(
        placeSessionHoverCard(
          anchor,
          { width: window.innerWidth, height: window.innerHeight },
          { width: SESSION_HOVER_CARD_WIDTH_PX, height },
        ),
      );
    };
    measure();
  }, [anchor]);

  if (typeof document === "undefined" || !anchor) {
    return null;
  }

  const portalHost = host ?? document.body;
  const modelValue =
    summary === null || summary.status === "loading"
      ? t("workspace.sessionCardLoading")
      : summary.status === "ready"
        ? summary.model
        : t("workspace.sessionCardUnavailable");

  return createPortal(
    <div
      id={id}
      ref={cardRef}
      role="tooltip"
      className="session-hover-card"
      style={{
        top: placement?.top ?? anchor.top,
        left: placement?.left ?? anchor.right,
        visibility: placement ? "visible" : "hidden",
      }}
    >
      <p className="session-hover-card__title">{session.title}</p>
      <dl className="session-hover-card__facts">
        <Fact label={t("workspace.sessionCardStatus")} value={sessionStatusLabel(session.status, t)} />
        <Fact label={t("workspace.sessionCardUpdated")} value={formatUtcTimestamp(session.updatedAt)} />
        <Fact label={t("workspace.sessionCardCreated")} value={formatUtcTimestamp(session.createdAt)} />
        {session.parentSessionId ? (
          <>
            <Fact
              label={t("workspace.sessionCardForkedFrom")}
              value={parentTitle ?? shortId(session.parentSessionId)}
            />
            <Fact
              label={t("workspace.sessionCardForkPoint")}
              value={
                session.forkPointRunId
                  ? `${shortId(session.forkPointRunId)}${
                      session.forkPointSeq ? ` · ${t("workspace.sessionCardEvent", { seq: session.forkPointSeq })}` : ""
                    }`
                  : t("workspace.sessionCardUnavailable")
              }
            />
          </>
        ) : null}
        <Fact label={t("workspace.sessionCardWorkspaceRoot")} value={formatDisplayPath(workspaceRootPath)} />
        <Fact label={t("workspace.sessionCardModel")} value={modelValue} />
        {summary?.status === "ready" ? (
          <>
            <Fact
              label={t("workspace.sessionCardReasoning")}
              // Same wording as the session model control: the provider default
              // is a named state, explicit levels stay as the provider spells them.
              value={
                summary.reasoning === "default"
                  ? t("modelControl.providerDefault")
                  : summary.reasoning
              }
            />
            <Fact
              label={t("workspace.sessionCardMaxSteps")}
              value={String(summary.maxSteps)}
            />
          </>
        ) : null}
      </dl>
    </div>,
    portalHost,
  );
}

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <div className="session-hover-card__fact">
      <dt>{label}</dt>
      <dd>{value}</dd>
    </div>
  );
}
