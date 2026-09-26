// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { RichText } from "./RichText";

/**
 * Segment memoization (design F3).
 *
 * The render-count gate cannot be written against `renderToStaticMarkup`: a
 * server render never re-renders a component, so nothing about the memo is
 * observable there. This file mounts the real component in jsdom, replaces the
 * markdown parser with a call counter, and streams deltas into it.
 *
 * `segmentMarkdown` only splits at a closed fence, so a finished *block* only
 * exists once a fence has closed. The fixture therefore contains one.
 */

const parser = vi.hoisted(() => ({ calls: [] as string[] }));

vi.mock("react-markdown", () => ({
  default: ({ children }: { children?: unknown }) => {
    parser.calls.push(String(children ?? ""));
    return <div className="parsed-segment">{String(children ?? "")}</div>;
  },
  // RichText imports it for safe URL handling; the real transform is a
  // pass-through for the URLs these cases use.
  defaultUrlTransform: (url: string) => url,
}));

const HEAD = "# Result\n\nFirst paragraph.\n\n```rust\nfn main() {}\n```\n\n";

function parsesStartingWith(prefix: string): number {
  return parser.calls.filter((text) => text.startsWith(prefix)).length;
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  parser.calls.length = 0;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => {
    root.unmount();
  });
  container.remove();
});

async function render(content: string) {
  await act(async () => {
    root.render(<RichText content={content} />);
  });
}

describe("RichText streaming memo", () => {
  it("renders every segment once on mount", async () => {
    await render(`${HEAD}streaming tail`);

    expect(parser.calls).toEqual([
      "# Result\n\nFirst paragraph.\n",
      "```rust\nfn main() {}\n```",
      // The blank line after the fence belongs to the tail segment.
      "\nstreaming tail",
    ]);
  });

  it("does not re-parse a finished block as deltas keep arriving", async () => {
    await render(`${HEAD}streaming tail`);
    expect(parsesStartingWith("# Result")).toBe(1);
    expect(parsesStartingWith("```rust")).toBe(1);
    const afterMount = parser.calls.length;

    for (const delta of [" one", " two", " three", " four", " five"]) {
      await render(`${HEAD}streaming tail${delta}`);
    }

    // Five more renders, and the two finished blocks were still parsed once.
    expect(parsesStartingWith("# Result")).toBe(1);
    expect(parsesStartingWith("```rust")).toBe(1);
    // Exactly one parse per delta: the growing tail.
    expect(parser.calls.length).toBe(afterMount + 5);
    expect(parser.calls.at(-1)).toBe("\nstreaming tail five");
  });

  it("re-parses the block a closing fence completes, once", async () => {
    await render("intro\n\n```rust\nfn main() {}\n");
    const asProse = parser.calls.length;
    expect(asProse).toBe(2);

    await render("intro\n\n```rust\nfn main() {}\n```\n");
    // The fence turned its prose block into a code block: the finished code
    // block is parsed once, the empty prose after the fence is new, and the
    // block before the fence is untouched.
    expect(parser.calls.length).toBe(asProse + 2);
    expect(parsesStartingWith("intro")).toBe(1);
    expect(parsesStartingWith("```rust")).toBe(2);
  });

  it("keeps the documented limit: prose-only streaming is one growing segment", async () => {
    // F3 does not split prose into top-level blocks (design §4.1), so a reply
    // without a fence still re-parses. Pinned here so the limit is deliberate.
    const prose = "# Result\n\nFirst paragraph.";
    await render(prose);
    expect(parser.calls).toEqual([prose]);

    await render(`${prose} More.`);
    expect(parser.calls).toEqual([prose, `${prose} More.`]);
  });
});
