import type { ReactNode } from "react";

import { ProductApp } from "../../shell/ProductApp";

export default function ProductLayout({ children }: { children: ReactNode }) {
  return (
    <>
      <ProductApp />
      {children}
    </>
  );
}
