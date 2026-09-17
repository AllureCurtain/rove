import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { createTranslator, type Locale } from "../copy";
import { PRODUCT_ARTIFACT_AVAILABILITIES, PRODUCT_ARTIFACT_SOURCE_KINDS, type ProductArtifactView } from "../product/product-api-types";
import { ArtifactPanel } from "./ArtifactPanel";

const state = vi.hoisted(() => ({ locale: "zh-CN" as Locale, artifacts: [] as ProductArtifactView[] }));
vi.mock("../copy/CopyProvider", () => ({
  useCopy: () => ({ t: createTranslator(state.locale) }),
}));
vi.mock("react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react")>();
  return {
    ...actual,
    useState: (initial: unknown) => actual.useState(Array.isArray(initial) ? state.artifacts : initial),
  };
});

describe.each(["zh-CN", "en-US"] as const)("Artifact copy (%s)", (locale) => {
  it("humanizes every source and availability while retaining download permissions", () => {
    state.locale = locale;
    state.artifacts = PRODUCT_ARTIFACT_SOURCE_KINDS.flatMap((source_kind) =>
      PRODUCT_ARTIFACT_AVAILABILITIES.map((availability, index) => ({
        artifact_id: `${source_kind}-${availability}`, safe_name: `file-${index}.txt`,
        mime: "text/plain", source_run_id: "internal-run-id", source_kind,
        availability, preview_kind: "text" as const, size: 1024,
      })),
    );
    const html = renderToStaticMarkup(<ArtifactPanel sessionId="session-1" />);
    const t = createTranslator(locale);
    for (const kind of PRODUCT_ARTIFACT_SOURCE_KINDS) expect(html).toContain(t(`artifactLabels.${kind}`));
    for (const availability of PRODUCT_ARTIFACT_AVAILABILITIES) {
      expect(html).toContain(t(`artifactLabels.${availability}`));
      expect(html).toContain(`data-availability="${availability}"`);
    }
    expect(html.match(/title="(?:Download|下载) /g)).toHaveLength(10);
    expect(html).toContain("1.0 KiB");
    const text = html.replace(/<[^>]+>/g, "");
    expect(text).not.toMatch(/task_state|tool_artifact|too_large|internal-run-id|registered/);
    if (locale === "zh-CN") expect(text).not.toMatch(/available|cleaned|invalid|report|trace/);
  });
});
