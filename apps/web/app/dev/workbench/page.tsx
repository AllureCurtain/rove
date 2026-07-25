"use client";

import { RoveWorkbench } from "../../../components/rove-workbench";

/**
 * Advanced/Developer migration scaffold for the old workbench.
 * Not a primary product entry — default `/` is the product shell.
 * Prefer Settings → Advanced for Benchmark; this route is escape-hatch only.
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
        . Benchmark lives under Settings → Advanced.
      </div>
      <RoveWorkbench />
    </div>
  );
}
