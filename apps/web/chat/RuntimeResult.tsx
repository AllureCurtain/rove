"use client";

import { useId, useState } from "react";
import { ChevronDownIcon } from "@radix-ui/react-icons";
import { useCopy } from "../copy/CopyProvider";
import type { FinalizationRecord } from "../lib/rove-types";

const outcomeKeys: Record<string, string> = {
  success: "chat.resultSuccess",
  partial: "chat.resultPartial",
  blocked: "chat.resultBlocked",
  rejected: "chat.resultRejected",
  cancelled: "chat.resultCancelled",
  interrupted: "chat.resultInterrupted",
  exhausted: "chat.resultExhausted",
  indeterminate: "chat.resultIndeterminate",
  failed: "chat.resultFailed",
};

export function RuntimeResult({ record, content }: {
  record: FinalizationRecord;
  content: string;
}) {
  const { t } = useCopy();
  const incomplete = record.incomplete_step_ids?.length ?? 0;
  const succeeded = record.outcome === "success" && incomplete === 0;
  const [open, setOpen] = useState(!succeeded);
  const bodyId = useId();
  const outcomeKey = record.outcome === "success" && incomplete > 0
    ? "chat.resultUnknown"
    : outcomeKeys[record.outcome ?? ""] ?? "chat.resultUnknown";
  return (
    <section className="runtime-result" aria-label={t("chat.resultLabel")}>
      <p className="runtime-result__outcome">{t(outcomeKey)}</p>
      <p className="runtime-result__note">
        {t(record.mode === "deterministic_fallback" ? "chat.resultFallback" : "chat.resultRecorded")}
      </p>
      {incomplete > 0 ? <p>{t("chat.resultIncomplete", { count: incomplete })}</p> : null}
      <button type="button" className="runtime-result__toggle" aria-expanded={open}
        aria-controls={bodyId} onClick={() => setOpen((value) => !value)}>
        <ChevronDownIcon aria-hidden="true" data-open={open} />
        {t("chat.resultDetails")}
      </button>
      {open ? <pre id={bodyId} tabIndex={0} className="runtime-result__details">{content}</pre> : null}
    </section>
  );
}
