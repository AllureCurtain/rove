import { useEffect, useState } from "react";
import { useCopy } from "../copy/CopyProvider";

/**
 * §5.5 session-switch settle veil.
 *
 * A cold session switch covers the transcript — and only the transcript — with
 * an opaque veil so the reader never watches an empty scroller repopulate. The
 * skeleton rows are anchored to the bottom because that is where a session
 * opens; user-side rows sit on the right, matching the real geometry so the
 * reveal is a dissolve, not a jump.
 *
 * `settling` is the restore-in-flight flag. When it clears, the veil stays
 * mounted through one fade so the switch reads as content arriving under it
 * rather than a cut.
 */
const FADE_MS = 300;

/** Bottom-up: the last row sits closest to the composer reserve. */
const SKELETON_ROWS: ReadonlyArray<{ role: "user" | "assistant"; width: string }> = [
  { role: "assistant", width: "72%" },
  { role: "user", width: "38%" },
  { role: "assistant", width: "84%" },
  { role: "assistant", width: "56%" },
  { role: "user", width: "30%" },
  { role: "assistant", width: "66%" },
];

export function SessionSettleVeil({ settling }: { settling: boolean }) {
  const { t } = useCopy();
  const [mounted, setMounted] = useState(settling);
  const fading = mounted && !settling;

  useEffect(() => {
    if (settling) {
      setMounted(true);
      return;
    }
    if (!mounted) {
      return;
    }
    // One frame for the transcript to land its final geometry, then the fade.
    let timer = 0;
    const frame = requestAnimationFrame(() => {
      timer = window.setTimeout(() => setMounted(false), FADE_MS);
    });
    return () => {
      cancelAnimationFrame(frame);
      window.clearTimeout(timer);
    };
  }, [mounted, settling]);

  if (!mounted) {
    return null;
  }
  return (
    <div
      className="session-settle-veil"
      data-fading={fading || undefined}
      role="status"
      aria-label={t("chat.restoring")}
    >
      <div className="transcript-skeleton" aria-hidden="true">
        {SKELETON_ROWS.map((row, index) => (
          <div
            key={index}
            className="transcript-skeleton-row"
            data-role={row.role}
            style={{ width: row.width }}
          />
        ))}
      </div>
    </div>
  );
}
