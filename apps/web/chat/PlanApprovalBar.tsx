"use client";

import {
  CheckIcon,
  ChevronDownIcon,
  Cross2Icon,
  LockClosedIcon,
} from "@radix-ui/react-icons";
import { useState } from "react";

import { useCopy } from "../copy/CopyProvider";
import type { TaskPlan } from "../lib/rove-types";
import type { ProductApprovalPreference } from "../product/product-api-types";

/** One approve-mode the server advertised on the plan gate. */
export type PlanApprovalMode = ProductApprovalPreference;

export interface PlanApprovalGate {
  approvalId: string;
  plan: TaskPlan;
  offeredModes: string[];
}

function isApprovalPreference(mode: string): mode is ProductApprovalPreference {
  return mode === "ask" || mode === "auto" || mode === "never";
}

/**
 * The pending-plan gate at the head of the composer stack (design §6): when a
 * run's drafted plan is waiting on approval, the bar shows the goal and the
 * step list and splits approval into the advertised modes — approving also
 * sets how the run's steps may mutate the workspace for the rest of the turn.
 * Choosing an approve-mode persists it as the default approval policy; the
 * backend still enforces `offered_modes` server-side, so an unadvertised mode
 * can never be submitted from here. Renders nothing when no gate is pending.
 */
export function PlanApprovalBar({
  gate,
  busy,
  onDecision,
}: {
  gate: PlanApprovalGate | null;
  busy: boolean;
  onDecision: (approvalId: string, mode: "reject" | PlanApprovalMode) => void;
}) {
  const { t } = useCopy();
  const [open, setOpen] = useState(true);
  if (!gate) {
    return null;
  }
  const approveModes = gate.offeredModes.filter(isApprovalPreference);
  const modeLabel = (mode: PlanApprovalMode) =>
    mode === "ask"
      ? t("chat.planApprovalApproveAsk")
      : mode === "auto"
        ? t("chat.planApprovalApproveAuto")
        : mode;
  return (
    <section
      className="plan-approval-bar"
      aria-label={t("chat.planApprovalTitle")}
      role="alert"
    >
      <button
        type="button"
        className="plan-approval-bar__header"
        aria-expanded={open}
        onClick={() => setOpen((current) => !current)}
      >
        <LockClosedIcon aria-hidden="true" />
        <span className="plan-approval-bar__title">
          {t("chat.planApprovalTitle")}
        </span>
        <span className="plan-approval-bar__count">
          {t("chat.planApprovalSteps", { count: gate.plan.steps.length })}
        </span>
        <ChevronDownIcon
          className="plan-approval-bar__chevron"
          aria-hidden="true"
        />
      </button>
      {open ? (
        <div className="plan-approval-bar__body">
          {gate.plan.goal ? (
            <p className="plan-approval-bar__goal">{gate.plan.goal}</p>
          ) : null}
          <ol className="plan-approval-bar__steps">
            {gate.plan.steps.map((step, index) => (
              <li key={step.id}>
                {index + 1}. {step.title}
              </li>
            ))}
          </ol>
          <p className="plan-approval-bar__hint">
            {t("chat.planApprovalPersistHint")}
          </p>
        </div>
      ) : null}
      <div className="plan-approval-bar__actions field-actions">
        {busy ? (
          <span role="status">{t("inspector.approvalSubmitting")}</span>
        ) : null}
        {approveModes.map((mode) => (
          <button
            key={mode}
            type="button"
            disabled={busy}
            onClick={() => onDecision(gate.approvalId, mode)}
          >
            <CheckIcon />
            {modeLabel(mode)}
          </button>
        ))}
        <button
          type="button"
          className="danger"
          disabled={busy}
          onClick={() => onDecision(gate.approvalId, "reject")}
        >
          <Cross2Icon />
          {t("chat.reject")}
        </button>
      </div>
    </section>
  );
}
