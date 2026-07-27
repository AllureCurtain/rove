"use client";

import {
  CheckCircledIcon,
  Cross2Icon,
  ExclamationTriangleIcon,
} from "@radix-ui/react-icons";
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";

import {
  readM1BrowserMigrationState,
  runM1BrowserMigration,
  type MigrationStorage,
  type M1BrowserMigrationRunResult,
} from "../product/m1-browser-migration";
import { M1_BROWSER_STORAGE_KEYS } from "../product/m1-storage-keys";
import type {
  M1BrowserMigrationResponse,
  M1MigrationIssueCode,
} from "../product/product-api-types";
import {
  parseProductRoute,
  sessionHref,
  workspaceHref,
} from "../state/product-route";

type MigrationAttentionResult = Exclude<
  M1BrowserMigrationRunResult,
  { status: "not_needed" | "complete" }
>;

type MigrationGateState =
  | { status: "checking" | "migrating" }
  | {
      status: "ready";
      result: Extract<
        M1BrowserMigrationRunResult,
        { status: "not_needed" | "complete" }
      >;
      showCompleteNotice: boolean;
    }
  | { status: "attention"; result: MigrationAttentionResult };

interface MigrationAttentionContent {
  title: string;
  detail: string;
  action: string;
}

const MIGRATION_ISSUE_LABELS: Record<M1MigrationIssueCode, string> = {
  invalid_workspace: "Workspace could not be imported",
  missing_workspace: "Session workspace was missing",
  invalid_runtime_hint: "Runtime history could not be verified",
  ambiguous_runtime_binding: "Runtime history matched more than once",
  runtime_binding_not_found: "Runtime history was not found",
  invalid_preference_reference: "A saved preference reference was unavailable",
  preference_write_conflict: "Newer server preferences were preserved",
};

const SUPERSEDED_RECHECK_DELAY_MS = 100;
const MIGRATION_NOTICE_HANDOFF_KEY =
  "rove.product.migration.web-m1.notice.v1";

export function M1MigrationGate({ children }: { children: ReactNode }) {
  const [state, setState] = useState<MigrationGateState>({ status: "checking" });
  const generationRef = useRef(0);
  const inFlightRef = useRef<Promise<M1BrowserMigrationRunResult> | null>(null);
  const completedInThisMountRef = useRef(false);

  const migrate = useCallback(async () => {
    const generation = ++generationRef.current;
    let storage: MigrationStorage;
    try {
      storage = window.localStorage;
      setState({ status: migrationStartStatus(storage) });
    } catch {
      if (generationRef.current === generation) {
        setState({ status: "attention", result: storageUnavailableResult() });
      }
      return;
    }

    let inFlight = inFlightRef.current;
    if (inFlight === null) {
      inFlight = runMigrationWithBoundedRecheck(storage);
      inFlightRef.current = inFlight;
    }

    let result: M1BrowserMigrationRunResult;
    try {
      result = await inFlight;
    } catch {
      result = unexpectedMigrationFailure();
    } finally {
      if (inFlightRef.current === inFlight) {
        inFlightRef.current = null;
      }
    }
    if (result.status === "complete" && !result.reused) {
      completedInThisMountRef.current = true;
    }
    if (generationRef.current !== generation) {
      return;
    }
    if (result.status === "not_needed" || result.status === "complete") {
      if (
        result.status === "complete" &&
        rewriteLegacyProductRoute(
          result.state.acknowledgement,
          completedInThisMountRef.current,
        )
      ) {
        return;
      }
      setState({
        status: "ready",
        result,
        showCompleteNotice:
          result.status === "complete" &&
          (completedInThisMountRef.current ||
            consumeMigrationNoticeHandoff(result.state.acknowledgement)),
      });
    } else {
      setState({ status: "attention", result });
    }
  }, []);

  useEffect(() => {
    void migrate();
    return () => {
      ++generationRef.current;
    };
  }, [migrate]);

  if (state.status === "ready") {
    return (
      <div className="migration-ready">
        {state.result.status === "complete" && state.showCompleteNotice ? (
          <MigrationCompleteNotice result={state.result} />
        ) : null}
        {children}
      </div>
    );
  }

  return (
    <main className="migration-gate" aria-busy={state.status !== "attention"}>
      <div className="migration-gate__rail" aria-hidden="true">
        <span data-active="true" />
        <span data-active={state.status === "attention"} />
        <span />
      </div>
      {state.status !== "attention" ? (
        <section className="migration-gate__content" role="status" aria-live="polite">
          <p className="eyebrow">Continuity check</p>
          <h1>
            {state.status === "migrating"
              ? "Importing your workspace history"
              : "Preparing your workspace history"}
          </h1>
          <p>
            {state.status === "migrating"
              ? "Moving browser-saved workspaces and sessions into durable server storage."
              : "Checking saved browser data before reading the server catalog."}
          </p>
        </section>
      ) : (
        <MigrationAttention result={state.result} onRetry={() => void migrate()} />
      )}
    </main>
  );
}

function MigrationAttention({
  result,
  onRetry,
}: {
  result: MigrationAttentionResult;
  onRetry: () => void;
}) {
  const content = migrationAttentionContent(result);

  return (
    <section className="migration-gate__content" role="alert">
      <p className="eyebrow">Continuity check</p>
      <div className="migration-gate__heading">
        <ExclamationTriangleIcon aria-hidden="true" />
        <h1>{content.title}</h1>
      </div>
      <p>{content.detail}</p>
      <p className="migration-gate__assurance">
        Rove has not deleted your browser-saved workspaces, sessions, or provider
        references, and will not replace them with an empty server catalog.
      </p>
      <button type="button" onClick={onRetry} autoFocus>
        {content.action}
      </button>
    </section>
  );
}

function MigrationCompleteNotice({
  result,
}: {
  result: Extract<M1BrowserMigrationRunResult, { status: "complete" }>;
}) {
  const [visible, setVisible] = useState(true);
  const acknowledgement = result.state.acknowledgement;
  const hasIssues = acknowledgement.issues.length > 0;
  const importedCount =
    acknowledgement.workspace_mappings.length +
    acknowledgement.session_mappings.length +
    acknowledgement.provider_profile_mappings.length;
  return visible ? (
    <div
      className="migration-complete"
      data-tone={hasIssues ? "warning" : "success"}
      role="status"
      aria-live="polite"
    >
      {hasIssues ? (
        <ExclamationTriangleIcon aria-hidden="true" />
      ) : (
        <CheckCircledIcon aria-hidden="true" />
      )}
      <span>
        Browser data imported{importedCount > 0 ? ` (${importedCount} records)` : ""}.
        {hasIssues
          ? ` ${migrationIssueSummary(acknowledgement.issues.map((issue) => issue.code))}`
          : " Server storage is now authoritative."}
      </span>
      <button
        type="button"
        className="ghost icon-button"
        onClick={() => setVisible(false)}
        aria-label="Dismiss import summary"
      >
        <Cross2Icon />
      </button>
    </div>
  ) : null;
}

export function migrationAttentionContent(
  result: MigrationAttentionResult,
): MigrationAttentionContent {
  if (result.status === "pending") {
    return {
      title: "Import needs verification",
      detail:
        "The server may already have accepted this import. Checking again will replay the exact saved request without duplicating browser data.",
      action: "Verify import",
    };
  }
  if (result.status === "rejected") {
    return {
      title: "Saved data was not accepted",
      detail:
        "The server rejected this browser snapshot. Its pending receipt was cleared, so a later retry will validate a fresh snapshot.",
      action: "Retry import",
    };
  }
  if (result.status === "superseded") {
    return {
      title: "Import receipt changed",
      detail:
        "Another tab or a local storage change replaced the receipt while the server was applying this import.",
      action: "Check again",
    };
  }

  switch (result.failure.code) {
    case "invalid_legacy_state":
      return {
        title: "Saved browser data needs repair",
        detail:
          "The saved workspace catalog could not be validated. Correct or clear the invalid browser entry before checking again.",
        action: "Check again",
      };
    case "invalid_migration_state":
      return {
        title: "Import receipt needs repair",
        detail:
          "The saved import receipt is incomplete or invalid. Restore a valid receipt before checking again.",
        action: "Check again",
      };
    case "storage_write_failed":
      return {
        title: "Browser storage is unavailable",
        detail:
          "Rove could not safely persist or verify the import receipt. Allow this site to use browser storage, then check again.",
        action: "Check again",
      };
    case "lock_unavailable":
    case "lock_failed":
      return {
        title: "Exclusive browser access is unavailable",
        detail:
          "Rove needs a same-origin browser lock to prevent duplicate imports. Close other Rove tabs or use a browser that supports Web Locks, then check again.",
        action: "Check again",
      };
    case "request_failed":
    case "request_rejected":
    case "invalid_acknowledgement":
      return {
        title: "Import could not be verified",
        detail:
          "Rove could not establish a trusted server acknowledgement for the saved browser data.",
        action: "Check again",
      };
  }
}

export function migrationIssueSummary(codes: M1MigrationIssueCode[]): string {
  const labels = [...new Set(codes.map((code) => MIGRATION_ISSUE_LABELS[code]))];
  const count = codes.length;
  const prefix = `${count} item${count === 1 ? " needs" : "s need"} review`;
  return labels.length === 0 ? `${prefix}.` : `${prefix}: ${labels.join("; ")}.`;
}

export function mappedLegacyProductHref(
  pathname: string,
  acknowledgement: M1BrowserMigrationResponse,
): string | null {
  const route = parseProductRoute(pathname);
  if (route.kind !== "workspace" && route.kind !== "session") {
    return null;
  }
  const workspaceId = acknowledgement.workspace_mappings.find(
    (mapping) => mapping.source_id === route.workspaceId,
  )?.workspace_id;
  if (!workspaceId) {
    return null;
  }
  if (route.kind === "workspace") {
    return workspaceHref(workspaceId);
  }
  const sessionId = acknowledgement.session_mappings.find(
    (mapping) => mapping.source_id === route.sessionId,
  )?.product_session_id;
  return sessionId ? sessionHref(workspaceId, sessionId) : null;
}

function migrationStartStatus(
  storage: MigrationStorage,
): "checking" | "migrating" {
  try {
    const existing = readM1BrowserMigrationState(storage);
    if (existing?.status === "complete") {
      return "checking";
    }
    return Object.values(M1_BROWSER_STORAGE_KEYS).some(
      (key) => storage.getItem(key) !== null,
    )
      ? "migrating"
      : "checking";
  } catch {
    return "checking";
  }
}

function rewriteLegacyProductRoute(
  acknowledgement: M1BrowserMigrationResponse,
  preserveNotice: boolean,
): boolean {
  const href = mappedLegacyProductHref(window.location.pathname, acknowledgement);
  if (!href || href === window.location.pathname) {
    return false;
  }
  if (preserveNotice) {
    persistMigrationNoticeHandoff(acknowledgement);
  }
  window.location.replace(`${href}${window.location.search}${window.location.hash}`);
  return true;
}

function persistMigrationNoticeHandoff(
  acknowledgement: M1BrowserMigrationResponse,
): void {
  try {
    window.sessionStorage.setItem(
      MIGRATION_NOTICE_HANDOFF_KEY,
      JSON.stringify({
        idempotency_key: acknowledgement.idempotency_key,
        receipt_id: acknowledgement.receipt_id,
      }),
    );
  } catch {
    // Route recovery remains valid when an ephemeral success notice cannot persist.
  }
}

function consumeMigrationNoticeHandoff(
  acknowledgement: M1BrowserMigrationResponse,
): boolean {
  try {
    const raw = window.sessionStorage.getItem(MIGRATION_NOTICE_HANDOFF_KEY);
    if (raw === null) {
      return false;
    }
    window.sessionStorage.removeItem(MIGRATION_NOTICE_HANDOFF_KEY);
    const handoff = JSON.parse(raw) as {
      idempotency_key?: unknown;
      receipt_id?: unknown;
    };
    return (
      handoff.idempotency_key === acknowledgement.idempotency_key &&
      handoff.receipt_id === acknowledgement.receipt_id
    );
  } catch {
    return false;
  }
}

async function runMigrationWithBoundedRecheck(
  storage: MigrationStorage,
): Promise<M1BrowserMigrationRunResult> {
  const result = await runM1BrowserMigration({ storage });
  if (result.status !== "superseded") {
    return result;
  }

  await new Promise((resolve) =>
    window.setTimeout(resolve, SUPERSEDED_RECHECK_DELAY_MS),
  );
  try {
    const current = readM1BrowserMigrationState(storage);
    if (
      current?.status === "complete" &&
      current.idempotency_key === result.acknowledgement.idempotency_key &&
      current.acknowledgement.receipt_id === result.acknowledgement.receipt_id
    ) {
      return { status: "complete", state: current, reused: true };
    }
  } catch {
    // The typed superseded result remains authoritative.
  }
  return result;
}

function storageUnavailableResult(): MigrationAttentionResult {
  return {
    status: "blocked",
    failure: {
      code: "storage_write_failed",
      message: "browser storage is unavailable",
    },
  };
}

function unexpectedMigrationFailure(): MigrationAttentionResult {
  return {
    status: "blocked",
    failure: {
      code: "request_failed",
      message: "browser migration failed unexpectedly",
    },
  };
}
