"use client";

import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";

import { useCopy } from "../copy/CopyProvider";
import {
  READING_WIDTH_MAX,
  READING_WIDTH_MIN,
  clampReadingWidth,
  readingWidthFromDrag,
  readingWidthFromKeyboard,
  type ReadingWidthSide,
} from "./reading-width";
import { useReadingWidth } from "./use-reading-width";

type DragState = {
  pointerId: number;
  side: ReadingWidthSide;
  startClientX: number;
  startWidth: number;
  currentWidth: number;
  frame: number;
};

/**
 * Dual edge handles for the centered conversation band, ported from PI-Desktop's
 * `ConversationWidthHandles` (D439).
 *
 * Both edges move the same band, so a 1px pointer move changes the width by 2px.
 * `Escape` cancels an in-flight drag, double-click returns to the design measure,
 * and the whole document switches to the resize cursor while dragging. Unlike the
 * panel divider (a full-height rule) this affordance is a short capsule that only
 * appears on hover, focus, or drag — the reference's reason is that a transcript
 * edge is not a place to keep permanent chrome.
 *
 * The handles render inside `.chat-transcript-frame`, which is exactly as wide as
 * the conversation column, so the reference's surface-relative edge math holds.
 * The width itself is published as `--reading-column-max` on `.chat-pane`, so the
 * transcript *and* the composer keep sharing one measure.
 */
export function ReadingWidthHandles() {
  const { t } = useCopy();
  const hostRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<DragState | null>(null);
  const { preferred, commit, reset } = useReadingWidth();
  const [preview, setPreview] = useState<number | null>(null);
  const [available, setAvailable] = useState(0);

  const live = clampReadingWidth(preview ?? preferred, available);

  // Measure the band's real usable span instead of restating the CSS padding:
  // the transcript's horizontal padding is the gutter, and it differs on narrow
  // layouts.
  useEffect(() => {
    const host = hostRef.current;
    const transcript = host
      ?.closest<HTMLElement>(".chat-pane")
      ?.querySelector<HTMLElement>(".chat-transcript");
    if (!transcript) {
      return;
    }
    const sync = () => {
      const style = window.getComputedStyle(transcript);
      const padding =
        (Number.parseFloat(style.paddingLeft) || 0) +
        (Number.parseFloat(style.paddingRight) || 0);
      setAvailable(Math.max(0, transcript.clientWidth - padding));
    };
    sync();
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", sync);
      return () => window.removeEventListener("resize", sync);
    }
    const observer = new ResizeObserver(sync);
    observer.observe(transcript);
    return () => observer.disconnect();
  }, []);

  useLayoutEffect(() => {
    const pane = hostRef.current?.closest<HTMLElement>(".chat-pane");
    if (!pane || available <= 0) {
      return;
    }
    pane.style.setProperty("--reading-column-max", `${live}px`);
    return () => {
      // `removeProperty` returns the old value, which an effect cleanup must not
      // pass through.
      pane.style.removeProperty("--reading-column-max");
    };
  }, [available, live]);

  useEffect(() => {
    const root = document.documentElement;
    if (preview === null) {
      root.removeAttribute("data-reading-resizing");
      return;
    }
    root.setAttribute("data-reading-resizing", "true");
    return () => root.removeAttribute("data-reading-resizing");
  }, [preview]);

  const finishDrag = useCallback(
    (element: HTMLElement, pointerId: number, cancelled: boolean) => {
      const drag = dragRef.current;
      if (!drag || drag.pointerId !== pointerId) {
        return;
      }
      if (drag.frame) {
        cancelAnimationFrame(drag.frame);
      }
      dragRef.current = null;
      setPreview(null);
      if (element.hasPointerCapture?.(pointerId)) {
        element.releasePointerCapture(pointerId);
      }
      // A cancelled drag leaves the preference exactly as it was.
      if (!cancelled) {
        commit(drag.currentWidth, available);
      }
    },
    [available, commit],
  );

  // A mid-drag unmount must not leave the document flagged as resizing.
  useEffect(
    () => () => {
      const drag = dragRef.current;
      if (drag?.frame) {
        cancelAnimationFrame(drag.frame);
      }
      dragRef.current = null;
      document.documentElement.removeAttribute("data-reading-resizing");
    },
    [],
  );

  const onPointerDown = useCallback(
    (side: ReadingWidthSide) => (event: ReactPointerEvent<HTMLDivElement>) => {
      if (event.pointerType === "mouse" && event.button !== 0) {
        return;
      }
      // The drag starts from the width that is actually rendered, so the band
      // tracks the pointer even when the preference is clamped by the column.
      const startWidth = clampReadingWidth(preferred, available);
      dragRef.current = {
        pointerId: event.pointerId,
        side,
        startClientX: event.clientX,
        startWidth,
        currentWidth: startWidth,
        frame: 0,
      };
      setPreview(startWidth);
      event.currentTarget.setPointerCapture(event.pointerId);
    },
    [available, preferred],
  );

  const onPointerMove = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const drag = dragRef.current;
      if (!drag || drag.pointerId !== event.pointerId) {
        return;
      }
      drag.currentWidth = readingWidthFromDrag({
        side: drag.side,
        startWidth: drag.startWidth,
        startClientX: drag.startClientX,
        clientX: event.clientX,
        available,
      });
      if (drag.frame) {
        return;
      }
      drag.frame = requestAnimationFrame(() => {
        if (dragRef.current !== drag) {
          return;
        }
        drag.frame = 0;
        setPreview(drag.currentWidth);
      });
    },
    [available],
  );

  const onPointerUp = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      finishDrag(event.currentTarget, event.pointerId, false);
    },
    [finishDrag],
  );

  const onPointerCancel = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      finishDrag(event.currentTarget, event.pointerId, true);
    },
    [finishDrag],
  );

  const onDoubleClick = useCallback(() => {
    setPreview(null);
    reset(available);
  }, [available, reset]);

  const onKeyDown = useCallback(
    (side: ReadingWidthSide) => (event: ReactKeyboardEvent<HTMLDivElement>) => {
      const drag = dragRef.current;
      if (event.key === "Escape" && drag) {
        event.preventDefault();
        finishDrag(event.currentTarget, drag.pointerId, true);
        return;
      }
      const next = readingWidthFromKeyboard(event, live, { side, available });
      if (next === null) {
        return;
      }
      event.preventDefault();
      commit(next, available);
    },
    [available, commit, finishDrag, live],
  );

  const maximum = Math.max(
    READING_WIDTH_MIN,
    Math.min(READING_WIDTH_MAX, available),
  );
  const label = t("chat.readingWidth");
  const valueText = t("chat.readingWidthValue", { width: live });
  // No room to move means no control: the band is already `min(available, …)`.
  const showHandles = available >= READING_WIDTH_MIN;

  return (
    <div ref={hostRef} className="chat-reading-handles">
      {showHandles
        ? (["left", "right"] as const).map((side) => (
            <div
              key={side}
              className="chat-reading-handle"
              data-side={side}
              role="separator"
              aria-orientation="vertical"
              aria-label={label}
              aria-valuemin={READING_WIDTH_MIN}
              aria-valuemax={maximum}
              aria-valuenow={live}
              aria-valuetext={valueText}
              tabIndex={0}
              onPointerDown={onPointerDown(side)}
              onPointerMove={onPointerMove}
              onPointerUp={onPointerUp}
              onPointerCancel={onPointerCancel}
              onLostPointerCapture={onPointerCancel}
              onDoubleClick={onDoubleClick}
              onKeyDown={onKeyDown(side)}
            />
          ))
        : null}
    </div>
  );
}
