"use client";

import { RoveWorkbench } from "../../../components/rove-workbench";

/**
 * Advanced/Developer migration scaffold for the old workbench.
 * Not a primary product entry — default `/` is the product shell.
 * Escape-hatch route only: the benchmark runner is not a product Settings
 * section, so this banner points nowhere else.
 */
export default function WorkbenchDevPage() {
  return (
    <div>
      <div
        style={{
          padding: "10px 16px",
          borderBottom: "1px solid var(--border, rgba(26,31,28,0.12))",
          background: "var(--surface-soft, #eef0ed)",
          color: "var(--muted, #6b736e)",
          fontSize: "0.85rem",
        }}
      >
        Advanced scaffold — product shell is{" "}
        <a href="/" style={{ color: "var(--accent, #3a5f7a)" }}>
          /
        </a>
        . Development route only.
      </div>
      <RoveWorkbench />
    </div>
  );
}
