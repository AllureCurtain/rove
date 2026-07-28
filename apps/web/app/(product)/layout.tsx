import type { ReactNode } from "react";

import {
  ProductApp,
  type ProductUiVersion,
} from "../../shell/ProductApp";

function productUiVersion(): ProductUiVersion {
  return process.env.ROVE_PRODUCT_UI_VERSION === "v1" ? "v1" : "v2";
}

export default function ProductLayout({ children }: { children: ReactNode }) {
  return (
    <>
      <ProductApp uiVersion={productUiVersion()} />
      {children}
    </>
  );
}
