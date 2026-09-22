"use client";

import { useCallback, useEffect, useRef, useState } from "react";

import {
  WORK_PANEL_COMPACT_MIN_WIDTH,
  clampWorkPanelWidth,
  workPanelMinimumFor,
} from "./work-panel-layout";

type DragState = {
  pointerId: number;
  startClientX: number;
  startWidth: number;
  minimumWidth: number;
  currentWidth: number;
  frame: number;
};

/**
 * Pointer and keyboard resizing for the work panel separator.
 *
 * Mirrors PI-Desktop's `WorkPanel` divider: the drag is measured against its own
 * start point (the separator sits on the panel's leading edge), pointer moves are
 * coalesced through one animation frame, the committed width is written when the
 * drag settles, and Escape reverts an in-flight drag. A `data-work-panel-resizing`
 * attribute on the document element gives the whole window the resize cursor.
 */
export function usePanelResize({
  renderedWidth,
  maxPanelWidth,
  setWidth,
  resizeByKeyboard,
}: {
  /** Width the budget currently renders (never the raw request). */
  renderedWidth: number;
  /** Largest width the live three-column budget allows. */
  maxPanelWidth: number;
  setWidth: (width: number) => void;
  resizeByKeyboard: (
    event: { key: string; shiftKey: boolean; preventDefault: () => void },
    maxPanelWidth: number,
  ) => void;
}) {
  const [dragWidth, setDragWidth] = useState<number | null>(null);
  const dragRef = useRef<DragState | null>(null);

  useEffect(() => {
    const root = document.documentElement;
    if (dragWidth === null) {
      root.removeAttribute("data-work-panel-resizing");
      return;
    }
    root.setAttribute("data-work-panel-resizing", "true");
    return () => root.removeAttribute("data-work-panel-resizing");
  }, [dragWidth]);

  const finish = useCallback(
    (element: HTMLElement, pointerId: number, cancel: boolean) => {
      const drag = dragRef.current;
      if (!drag || drag.pointerId !== pointerId) {
        return;
      }
      if (drag.frame) {
        cancelAnimationFrame(drag.frame);
      }
      dragRef.current = null;
      setDragWidth(null);
      if (element.hasPointerCapture?.(pointerId)) {
        element.releasePointerCapture(pointerId);
      }
      // A cancelled drag leaves the stored width exactly as it was.
      if (!cancel) {
        setWidth(drag.currentWidth);
      }
    },
    [setWidth],
  );

  // A mid-drag unmount must not leave the document flagged as resizing.
  useEffect(
    () => () => {
      const drag = dragRef.current;
      if (drag?.frame) {
        cancelAnimationFrame(drag.frame);
      }
      dragRef.current = null;
      document.documentElement.removeAttribute("data-work-panel-resizing");
    },
    [],
  );

  const onPointerDown = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      // Pointer-only affordance; keyboard resizing is handled below.
      if (event.pointerType === "mouse" && event.button !== 0) {
        return;
      }
      const minimumWidth = workPanelMinimumFor(renderedWidth);
      const startWidth = clampWorkPanelWidth(renderedWidth, minimumWidth);
      dragRef.current = {
        pointerId: event.pointerId,
        startClientX: event.clientX,
        startWidth,
        minimumWidth,
        currentWidth: startWidth,
        frame: 0,
      };
      setDragWidth(startWidth);
      event.currentTarget.setPointerCapture(event.pointerId);
    },
    [renderedWidth],
  );

  const onPointerMove = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (drag?.pointerId !== event.pointerId) {
      return;
    }
    drag.currentWidth = clampWorkPanelWidth(
      drag.startWidth + drag.startClientX - event.clientX,
      drag.minimumWidth,
    );
    if (drag.frame) {
      return;
    }
    drag.frame = requestAnimationFrame(() => {
      if (dragRef.current !== drag) {
        return;
      }
      drag.frame = 0;
      setDragWidth(drag.currentWidth);
    });
  }, []);

  const onPointerUp = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      finish(event.currentTarget, event.pointerId, false);
    },
    [finish],
  );

  const onPointerCancel = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      finish(event.currentTarget, event.pointerId, true);
    },
    [finish],
  );

  const onKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      const drag = dragRef.current;
      if (event.key === "Escape" && drag) {
        event.preventDefault();
        finish(event.currentTarget, drag.pointerId, true);
        return;
      }
      resizeByKeyboard(event, maxPanelWidth);
    },
    [finish, maxPanelWidth, resizeByKeyboard],
  );

  const value = dragWidth ?? renderedWidth;
  const minimumWidth = workPanelMinimumFor(value);
  return {
    /** Live preview width while dragging, else null. */
    dragWidth,
    value,
    minimumWidth: Math.min(
      minimumWidth,
      Math.max(WORK_PANEL_COMPACT_MIN_WIDTH, maxPanelWidth),
    ),
    maximumWidth: Math.max(minimumWidth, Math.round(maxPanelWidth)),
    onPointerDown,
    onPointerMove,
    onPointerUp,
    onPointerCancel,
    onKeyDown,
  };
}
