"use client";

import { FileIcon } from "@radix-ui/react-icons";
import type { ReactNode } from "react";

/**
 * The one empty state every work-panel tab shares (design §8): an icon, a
 * title, and one line of guidance — never a blank pane. The icon slot defaults
 * to a file glyph so callers only override when a kind needs its own.
 */
export function InspectorEmpty({
  icon,
  title,
  body,
}: {
  icon?: ReactNode;
  title: string;
  body: string;
}) {
  return (
    <div className="inspector-empty" role="status">
      <span className="inspector-empty__icon" aria-hidden="true">
        {icon ?? <FileIcon />}
      </span>
      <strong className="inspector-empty__title">{title}</strong>
      <p className="inspector-empty__body">{body}</p>
    </div>
  );
}
