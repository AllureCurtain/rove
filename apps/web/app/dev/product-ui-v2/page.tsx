import type { Metadata } from "next";

import { ProductUiV2Preview } from "./ProductUiV2Preview";
import { PRODUCT_UI_V2_PREVIEW_BOUNDARY } from "./product-ui-v2-mock";

export const metadata: Metadata = {
  title: "Rove Product UI V2 Inert Mock Preview",
  description: PRODUCT_UI_V2_PREVIEW_BOUNDARY,
  robots: {
    index: false,
    follow: false,
  },
};

export default function ProductUiV2PreviewPage() {
  return <ProductUiV2Preview />;
}
