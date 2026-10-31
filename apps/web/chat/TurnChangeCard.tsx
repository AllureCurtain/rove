"use client";

import {
  CheckIcon,
  ChevronDownIcon,
  CopyIcon,
} from "@radix-ui/react-icons";
import { useEffect, useRef, useState } from "react";

import { useCopy } from "../copy/CopyProvider";
import { DiffView } from "../product-v2/DiffView";
import type { ToolCallView } from "../lib/rove-state";
import type { ToolMutation, ToolMutationOperation } from "../lib/rove-types";
import { trimDiff } from "./tool-presentation";

/**
 * One file mutation inside a turn (issue #29, design §5.4): a dedicated
 * change card that replaces the generic tool card's diff block for the
 * file-mutating calls. The head is one line — A/M/D mark, the path, and the
 * +/− counts — and the body expands to the bounded diff hunks. A rollback
 * affordance is wired only when the product exposes a revert endpoint; it does
 * not today (the runtime's `workspace_rewind` is a model tool over its own
 * process-local checkpoints, not a session revert API — see issue #29), so the
 * card says rollback is unavailable rather than offering a dead action.
 */

/** Git-status letters carry the operation; the word stays in the a11y name. */
const OPERATION_MARKS: Record<ToolMutationOperation, string> = {
  create: "A",
  update: "M",
  delete: "D",
  unknown: "?",
};

const OPERATION_LABEL_KEYS: Record<ToolMutationOperation, string> = {
  create: "chat.changeCreate",
  update: "chat.changeUpdate",
  delete: "chat.changeDelete",
  unknown: "chat.changeUnknown",
};

/**
 * Count added/deleted lines in a unified diff. `+++`/`---` headers are
 * excluded; a hunk body's `+`/`-` lines are what changed.
 */
export function diffLineCounts(diff: string): { additions: number; deletions: number } {
  let additions = 0;
  let deletions = 0;
  for (const line of diff.split("\n")) {
    if (line.startsWith("+") && !line.startsWith("+++")) {
      additions += 1;
    } else if (line.startsWith("-") && !line.startsWith("---")) {
      deletions += 1;
    }
  }
  return { additions, deletions };
}

function operationStatusKey(operation: ToolMutationOperation): string {
  return OPERATION_LABEL_KEYS[operation] ?? "chat.changeUnknown";
}

export function TurnChangeCard({
  tool,
  mutation,
  onOpenFile,
}: {
  tool: ToolCallView;
  mutation: ToolMutation;
  /** Clicking the path opens the file tab; the diff stays in this card. */
  onOpenFile?: (reference: { path: string; line?: number }) => void;
}) {
  const { t } = useCopy();
  const [open, setOpen] = useState(
    tool.status === "running" || tool.status === "error",
  );
  const [copied, setCopied] = useState(false);
  const copyResetRef = useRef<number | null>(null);
  useEffect(
    () => () => {
      if (copyResetRef.current !== null) {
        window.clearTimeout(copyResetRef.current);
      }
    },
    [],
  );
  const detailsId = `change-card-${tool.timelineId ?? tool.id}-${mutation.path}`.replace(
    /[^A-Za-z0-9_-]/gu,
    "-",
  );
  const trimmed = mutation.diff ? trimDiff(mutation.diff) : null;
  const counts = mutation.diff
    ? diffLineCounts(trimmed ? trimmed.text : mutation.diff)
    : { additions: 0, deletions: 0 };
  const statusLabel = t(operationStatusKey(mutation.operation));
  const diffTruncated = trimmed?.truncated ?? false;

  async function copyDiff() {
    if (!mutation.diff) {
      return;
    }
    try {
      await navigator.clipboard.writeText(mutation.diff);
      setCopied(true);
      if (copyResetRef.current !== null) {
        window.clearTimeout(copyResetRef.current);
      }
      copyResetRef.current = window.setTimeout(() => {
        setCopied(false);
        copyResetRef.current = null;
      }, 1_500);
    } catch {
      setCopied(false);
    }
  }

  return (
    <article
      className="change-card"
      data-status={mutation.operation}
      data-tool-status={tool.status}
    >
      <div className="change-card__row">
        <button
          type="button"
          className="change-card__head"
          aria-controls={mutation.diff ? detailsId : undefined}
          aria-expanded={mutation.diff ? open : undefined}
          aria-label={t("chat.changeCardLabel", {
            status: statusLabel,
            path: mutation.path,
            additions: counts.additions,
            deletions: counts.deletions,
          })}
          title={`${statusLabel} · ${mutation.path}`}
          disabled={!mutation.diff}
          onClick={() => mutation.diff && setOpen((value) => !value)}
        >
          <ChevronDownIcon data-open={open} />
          <span
            className="change-card__mark"
            data-status={mutation.operation}
            aria-hidden="true"
          >
            {OPERATION_MARKS[mutation.operation] ?? "?"}
          </span>
        </button>
        {onOpenFile ? (
          <button
            type="button"
            className="change-card__path change-card__path--link"
            title={t("chat.openFileRef")}
            onClick={(event) => {
              event.stopPropagation();
              onOpenFile({ path: mutation.path });
            }}
          >
            {mutation.path}
          </button>
        ) : (
          <code className="change-card__path">{mutation.path}</code>
        )}
        <button
          type="button"
          className="change-card__tail"
          aria-label={t("chat.changeCardLabel", {
            status: statusLabel,
            path: mutation.path,
            additions: counts.additions,
            deletions: counts.deletions,
          })}
          disabled={!mutation.diff}
          onClick={() => mutation.diff && setOpen((value) => !value)}
        >
          <span className="change-card__counts">
            {counts.additions > 0 ? (
              <span className="change-card__count" data-kind="add">
                +{counts.additions}
              </span>
            ) : null}
            {counts.deletions > 0 ? (
              <span className="change-card__count" data-kind="del">
                −{counts.deletions}
              </span>
            ) : null}
          </span>
          <span className="tool-card__state" data-status={tool.status}>
            {tool.status === "running" ? (
              <span className="tool-card__spinner" aria-hidden="true" />
            ) : (
              <i className="tool-card__state-dot" aria-hidden="true" />
            )}
            <span>{toolStatusText(tool.status, t)}</span>
          </span>
        </button>
        {mutation.diff ? (
          <button
            type="button"
            className="tool-card__copy icon-button"
            aria-label={copied ? t("chat.messageCopied") : t("chat.changeCopy")}
            title={copied ? t("chat.messageCopied") : t("chat.changeCopy")}
            onClick={() => void copyDiff()}
          >
            {copied ? <CheckIcon /> : <CopyIcon />}
          </button>
        ) : null}
      </div>
      {open && mutation.diff ? (
        <div className="change-card__details" id={detailsId}>
          <DiffView
            diff={trimmed ? trimmed.text : mutation.diff}
            label={`${mutation.path} diff`}
            sourcePath={mutation.path}
          />
          {diffTruncated ? (
            <p className="tool-card__truncated">{t("chat.diffTruncated")}</p>
          ) : null}
          {/* Rollback is gated on a product revert endpoint that does not
              exist yet (issue #29); surface the state honestly rather than
              offering an action that cannot land. */}
          <p className="change-card__rollback-note" role="note">
            {t("chat.rollbackUnavailable")}
          </p>
        </div>
      ) : null}
    </article>
  );
}

function toolStatusText(
  status: ToolCallView["status"],
  t: (path: string) => string,
): string {
  switch (status) {
    case "running":
      return t("chat.toolStateRunning");
    case "waiting":
      return t("chat.toolStateWaiting");
    case "error":
      return t("chat.toolStateError");
    default:
      return t("chat.toolStateDone");
  }
}
