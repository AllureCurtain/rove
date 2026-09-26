"use client";

import { useEffect, useRef } from "react";

import { useCopy } from "../../copy/CopyProvider";
import { useToast } from "./ToastProvider";
import { newBackgroundFailures, type BackgroundSessionLike } from "./notifications";

/**
 * A session that fails while nobody is watching it still has to say so. The
 * sidebar dot is the only other sign, and it does not distinguish a failure the
 * reader has not seen from one they have.
 *
 * Only a change into failure is announced: a session already failed when this
 * window opened is not news, and the active session reports itself inline.
 * Repeats inside the toast dedupe window are the store's business.
 */
export function useBackgroundFailureToasts({
  sessions,
  activeSessionId,
}: {
  sessions: ReadonlyArray<BackgroundSessionLike>;
  activeSessionId: string | null;
}): void {
  const { t } = useCopy();
  const { notify } = useToast();
  const known = useRef<Map<string, string>>(new Map());

  useEffect(() => {
    const { failures, known: next } = newBackgroundFailures(
      known.current,
      sessions,
      activeSessionId,
    );
    known.current = next;
    for (const failure of failures) {
      notify({
        kind: "error",
        message: t("toast.sessionFailed", { title: failure.title }),
        dedupeKey: `session-failed:${failure.id}`,
      });
    }
  }, [sessions, activeSessionId, notify, t]);
}
