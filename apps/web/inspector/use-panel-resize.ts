"use client";

import { useCallback, useEffect, useRef } from "react";

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
};

/** The live drag preview rides on the shell's grid track variable. */
const PREVIEW_VARIABLE = "--work-panel-preview";
const RESIZING_ATTRIBUTE = "data-work-panel-resizing";

function writePreview(width: number | null): void {
  const root = document.documentElement;
  if (width === null) {
    root.style.removeProperty(PREVIEW_VARIABLE);
  } else {
    root.style.setProperty(PREVIEW_VARIABLE, `${Math.round(width)}px`);
  }
}

function setResizingFlag(active: boolean): void {
  const root = document.documentElement;
  if (active) {
    root.setAttribute(RESIZING_ATTRIBUTE, "true");
  } else {
    root.removeAttribute(RESIZING_ATTRIBUTE);
  }
}

/**
 * Pointer and keyboard resizing for the work panel separator.
 *
 * Mirrors PI-Desktop's `WorkPanel` divider and the left rail's committed
 * pattern: the drag is measured against its own start point (the separator
 * sits on the panel's leading edge), the preview is written straight to the
 * `--work-panel-preview` CSS variable — no per-frame React render — and the
 * committed width is stored only when the drag settles. Escape reverts an
 * in-flight drag. The document attribute gives the whole window the resize
 * cursor.
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
  const dragRef = useRef<DragState | null>(null);
  // The live budget cap moves as the shell re-measures; read it at drag time
  // through a ref so the preview respects the same ceiling the commit will.
  const maxPanelWidthRef = useRef(maxPanelWidth);
  maxPanelWidthRef.current = maxPanelWidth;

  const finish = useCallback(
    (element: HTMLElement, pointerId: number, cancel: boolean) => {
      const drag = dragRef.current;
      if (!drag || drag.pointerId !== pointerId) {
        return;
      }
      dragRef.current = null;
      writePreview(null);
      setResizingFlag(false);
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

  // A mid-drag unmount must not leave the document flagged as resizing or the
  // preview variable stuck at a width the stored value never took.
  useEffect(
    () => () => {
      if (dragRef.current) {
        dragRef.current = null;
      }
      writePreview(null);
      setResizingFlag(false);
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
      };
      setResizingFlag(true);
      writePreview(startWidth);
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
    // The budget's live cap still bounds the preview; the committed width runs
    // through the same clamp when the drag settles.
    const capped = Math.min(drag.currentWidth, Math.max(0, maxPanelWidthRef.current));
    writePreview(capped);
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

  const minimumWidth = workPanelMinimumFor(renderedWidth);
  return {
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
