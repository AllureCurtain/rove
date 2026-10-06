"use client";

import { CheckIcon, ChevronDownIcon, ListBulletIcon } from "@radix-ui/react-icons";
import { useState } from "react";

import { useCopy } from "../copy/CopyProvider";
import type { TaskPlan } from "../lib/rove-types";
import { todoDockSummary } from "./todo-dock";

/**
 * The session todo fold at the head of the composer stack (design §6): a
 * one-row header — symbol, "n/m done", the step the run is on — that expands
 * into the full step list. It renders nothing for a run that never produced a
 * plan, so the composer keeps its single-column shape on plain turns.
 */
export function TodoDock({ plan }: { plan: TaskPlan | null }) {
  const { t } = useCopy();
  const [open, setOpen] = useState(false);
  if (!plan || plan.steps.length === 0) {
    return null;
  }
  const summary = todoDockSummary(plan);
  return (
    <div className="todo-dock" data-open={open || undefined}>
      <button
        type="button"
        className="todo-dock__header"
        aria-expanded={open}
        aria-label={t("chat.todoDockLabel")}
        title={plan.goal}
        onClick={() => setOpen((current) => !current)}
      >
        <ListBulletIcon aria-hidden="true" />
        <span className="todo-dock__progress">
          {t("chat.todoDockProgress", {
            done: summary.done,
            total: summary.total,
          })}
        </span>
        <span className="todo-dock__current">
          {summary.current ? summary.current.title : t("chat.todoDockAllDone")}
        </span>
        <ChevronDownIcon className="todo-dock__chevron" aria-hidden="true" />
      </button>
      {open ? (
        <ul className="todo-dock__list">
          {plan.steps.map((step, index) => (
            <li
              key={step.id}
              data-done={step.done || undefined}
              data-current={
                summary.current?.id === step.id ? "true" : undefined
              }
            >
              <span className="todo-dock__mark" aria-hidden="true">
                {step.done ? <CheckIcon /> : null}
              </span>
              <span className="todo-dock__step-title">
                {index + 1}. {step.title}
              </span>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
