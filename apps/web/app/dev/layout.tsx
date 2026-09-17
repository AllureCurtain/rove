import { notFound } from "next/navigation";
import type { ReactNode } from "react";

function devRoutesEnabled(): boolean {
  return (
    process.env.ROVE_ENABLE_DEV_ROUTES === "1" ||
    process.env.NODE_ENV === "development"
  );
}

export default function DevRoutesLayout({ children }: { children: ReactNode }) {
  if (!devRoutesEnabled()) {
    notFound();
  }
  return children;
}
