/**
 * §7.3: Ctrl/Shift multi-select over the flat session list.
 *
 * The model is an anchor id plus a set of selected ids. A plain click does not
 * touch the selection at all — it navigates — so these helpers only run for
 * modified clicks. Shift ranges resolve against `order`, the visible row order
 * of the zone the click landed in.
 */
export interface SessionSelection {
  anchor: string | null;
  selected: ReadonlySet<string>;
}

export const EMPTY_SELECTION: SessionSelection = {
  anchor: null,
  selected: new Set<string>(),
};

export interface SelectionClick {
  /** Ctrl on Windows/Linux, Cmd on macOS — callers pass `ctrlKey || metaKey`. */
  additive: boolean;
  range: boolean;
}

/** True when the click carries a multi-select modifier. */
export function isSelectionClick(click: SelectionClick): boolean {
  return click.additive || click.range;
}

export function applySelectionClick(
  selection: SessionSelection,
  order: readonly string[],
  sessionId: string,
  click: SelectionClick,
): SessionSelection {
  if (click.range && selection.anchor !== null) {
    const anchorIndex = order.indexOf(selection.anchor);
    const targetIndex = order.indexOf(sessionId);
    if (anchorIndex !== -1 && targetIndex !== -1) {
      const [from, to] =
        anchorIndex <= targetIndex
          ? [anchorIndex, targetIndex]
          : [targetIndex, anchorIndex];
      return {
        anchor: selection.anchor,
        selected: new Set(order.slice(from, to + 1)),
      };
    }
    // Anchor or target fell out of the visible order: treat as a fresh anchor.
    return { anchor: sessionId, selected: new Set([sessionId]) };
  }
  if (click.additive) {
    const selected = new Set(selection.selected);
    if (selected.has(sessionId)) {
      selected.delete(sessionId);
    } else {
      selected.add(sessionId);
    }
    // The anchor is where a later Shift range extends from: it only moves
    // when the additive click is the first selection gesture.
    return { anchor: selection.anchor ?? sessionId, selected };
  }
  return { anchor: sessionId, selected: new Set([sessionId]) };
}

/** Drop ids that left the catalog (deleted, or archived out of view). */
export function pruneSelection(
  selection: SessionSelection,
  liveIds: ReadonlySet<string>,
): SessionSelection {
  if (
    selection.selected.size === 0 &&
    (selection.anchor === null || liveIds.has(selection.anchor))
  ) {
    return selection;
  }
  const selected = new Set(
    [...selection.selected].filter((id) => liveIds.has(id)),
  );
  const anchor =
    selection.anchor !== null && liveIds.has(selection.anchor)
      ? selection.anchor
      : null;
  if (selected.size === 0 && anchor === null) {
    return EMPTY_SELECTION;
  }
  return { anchor, selected };
}
