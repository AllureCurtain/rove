"use client";

import { useEffect, useMemo, useRef, useState } from "react";

import {
  buildCommandGroups,
  firstEnabledCommandId,
  flattenCommandGroups,
  moveCommandSelection,
  splitCommandHighlight,
  type CommandPaletteAction,
  type CommandPaletteEntry,
  type CommandPaletteGroupKey,
} from "./command-palette";

export interface CommandPaletteLabels {
  title: string;
  placeholder: string;
  empty: string;
  groups: Record<CommandPaletteGroupKey, string>;
}

/**
 * The command palette view (open-vetta's `CommandMenuView` adapted to this
 * shell).
 *
 * It performs no navigation itself: selecting an entry reports its action as
 * data, and the host runs it in one place. The list is a `listbox` driven from
 * the input through `aria-activedescendant`, so the first action after opening is
 * always typing and the keyboard never has to enter the list to reach a command.
 */
export function CommandPalette({
  open,
  entries,
  labels,
  onAction,
  onClose,
}: {
  open: boolean;
  entries: readonly CommandPaletteEntry[];
  labels: CommandPaletteLabels;
  onAction: (action: CommandPaletteAction) => void;
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);

  const groups = useMemo(
    () => buildCommandGroups(entries, query),
    [entries, query],
  );
  const items = useMemo(() => flattenCommandGroups(groups), [groups]);
  const selected = items.find((item) => item.entry.id === selectedId) ?? null;

  // Opening hands focus to the input: the next action after ⌘K is typing. A
  // fresh open also starts from an empty query rather than the last search.
  useEffect(() => {
    if (!open) {
      return;
    }
    restoreFocusRef.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    setQuery("");
    setSelectedId(null);
    const frame = window.requestAnimationFrame(() => inputRef.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, [open]);

  // Closing returns focus to whatever held it, so the palette is not a focus trap
  // the user has to escape with the mouse.
  useEffect(() => {
    if (open) {
      return;
    }
    restoreFocusRef.current?.focus();
    restoreFocusRef.current = null;
  }, [open]);

  // The selection follows the list: a query change must never leave the highlight
  // on a row that no longer exists.
  useEffect(() => {
    if (!open) {
      return;
    }
    if (selectedId && items.some((item) => item.entry.id === selectedId)) {
      return;
    }
    setSelectedId(firstEnabledCommandId(items));
  }, [items, open, selectedId]);

  if (!open) {
    return null;
  }

  function run(action: CommandPaletteAction, disabled: boolean | undefined) {
    if (disabled) {
      return;
    }
    onClose();
    onAction(action);
  }

  function handleKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
      return;
    }
    // A palette is a single-input surface; Tab stays on the input instead of
    // walking into a list the arrows already own.
    if (event.key === "Tab") {
      event.preventDefault();
      return;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      setSelectedId(
        moveCommandSelection(
          items,
          selectedId,
          event.key === "ArrowDown" ? 1 : -1,
        ),
      );
      return;
    }
    if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      const enabled = items.filter((item) => !item.entry.disabled);
      const target = event.key === "Home" ? enabled[0] : enabled.at(-1);
      setSelectedId(target?.entry.id ?? null);
      return;
    }
    if (event.key === "Enter" && selected) {
      event.preventDefault();
      run(selected.entry.action, selected.entry.disabled);
    }
  }

  return (
    <>
      <div className="command-palette-scrim" role="presentation" onClick={onClose} />
      <div
        className="command-palette"
        role="dialog"
        aria-modal="true"
        aria-label={labels.title}
        onKeyDown={handleKeyDown}
      >
        <input
          ref={inputRef}
          className="command-palette__input"
          type="text"
          role="combobox"
          aria-expanded="true"
          aria-controls="command-palette-list"
          aria-activedescendant={
            selected ? `command-palette-option-${selected.entry.id}` : undefined
          }
          aria-autocomplete="list"
          autoComplete="off"
          placeholder={labels.placeholder}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />

        <div
          className="command-palette__list"
          id="command-palette-list"
          role="listbox"
        >
          {items.length === 0 ? (
            <p className="command-palette__empty" role="status">
              {labels.empty}
            </p>
          ) : null}
          {groups.map((group) => (
            <div
              key={group.key}
              className="command-palette__group"
              role="group"
              aria-label={labels.groups[group.key]}
            >
              <p className="command-palette__group-title">
                {labels.groups[group.key]}
              </p>
              {group.items.map((item) => {
                const isSelected = item.entry.id === selectedId;
                return (
                  <div
                    key={item.entry.id}
                    id={`command-palette-option-${item.entry.id}`}
                    role="option"
                    aria-selected={isSelected}
                    aria-disabled={item.entry.disabled ? true : undefined}
                    className="command-palette__option"
                    data-selected={isSelected ? "true" : undefined}
                    data-disabled={item.entry.disabled ? "true" : undefined}
                    onPointerMove={() => {
                      if (!item.entry.disabled) {
                        setSelectedId(item.entry.id);
                      }
                    }}
                    onClick={() => run(item.entry.action, item.entry.disabled)}
                  >
                    <span className="command-palette__option-title">
                      {splitCommandHighlight(
                        item.entry.title,
                        item.match.titleRanges,
                      ).map((run, index) =>
                        run.highlighted ? (
                          <mark key={index}>{run.text}</mark>
                        ) : (
                          <span key={index}>{run.text}</span>
                        ),
                      )}
                    </span>
                    {item.entry.disabled && item.entry.disabledReason ? (
                      <span className="command-palette__option-note">
                        {item.entry.disabledReason}
                      </span>
                    ) : item.entry.subtitle ? (
                      <span className="command-palette__option-subtitle">
                        {item.entry.subtitle}
                      </span>
                    ) : null}
                  </div>
                );
              })}
              {group.hiddenCount > 0 ? (
                <p className="command-palette__more">+{group.hiddenCount}</p>
              ) : null}
            </div>
          ))}
        </div>
      </div>
    </>
  );
}
