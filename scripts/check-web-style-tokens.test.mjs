#!/usr/bin/env node
/**
 * Self-tests for the style-token linter (design F8). Run with:
 *
 *   node --test scripts/check-web-style-tokens.test.mjs
 *
 * The cases pin the rules rather than the current stylesheets, so the linter can
 * be trusted on the day it is asked to reject a new literal.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  collectDeclarations,
  findStyleTokenViolations,
  listExemptions,
  splitTopLevel,
  tokenize,
} from "./check-web-style-tokens.mjs";

const lines = (violations) => violations.map((violation) => violation.line);

test("tokenized durations and eases pass", () => {
  const css = `
.a {
  transition: opacity var(--motion-duration-fast) var(--motion-ease-out);
  animation-duration: var(--motion-duration-slow);
  animation-timing-function: var(--motion-ease-standard);
}
`;
  assert.deepEqual(findStyleTokenViolations(css), []);
});

test("a literal duration in a shorthand is reported at its declaration line", () => {
  const css = `.a {\n  transition: opacity 160ms var(--motion-ease-out);\n}\n`;
  const violations = findStyleTokenViolations(css);
  assert.equal(violations.length, 1);
  assert.deepEqual(lines(violations), [2]);
  assert.match(violations[0].message, /literal duration/u);
});

test("a literal timing function is reported, including cubic-bezier", () => {
  const css = `
.a {
  transition: color var(--motion-duration-fast) ease-out;
}
.b {
  animation: spin var(--motion-duration-spin) cubic-bezier(0.2, 0, 0, 1) infinite;
}
`;
  const violations = findStyleTokenViolations(css);
  assert.deepEqual(lines(violations), [3, 6]);
  assert.match(violations[0].message, /literal timing function/u);
  assert.match(violations[1].message, /cubic-bezier/u);
});

test("linear and steps() stay allowed, and a later time is a delay", () => {
  const css = `
.a {
  animation: v2-session-spin var(--motion-duration-spin) linear infinite;
  transition: opacity var(--motion-duration-fast) steps(2, end);
}
.b {
  transition: opacity var(--motion-duration-fast) var(--motion-ease-out) 150ms;
}
`;
  assert.deepEqual(findStyleTokenViolations(css), []);
});

test("a multi-line shorthand keeps one violation on its first line", () => {
  const css = `
.a {
  transition: transform 240ms var(--motion-ease-out),
    opacity var(--motion-duration-fast) var(--motion-ease-out);
}
`;
  const violations = findStyleTokenViolations(css);
  assert.deepEqual(lines(violations), [3]);
});

test("declarations inside prefers-reduced-motion are exempt", () => {
  const css = `
@media (prefers-reduced-motion: reduce) {
  .a {
    animation-duration: 0.01ms !important;
    transition-duration: 0.01ms !important;
  }
}
`;
  assert.deepEqual(findStyleTokenViolations(css), []);
});

test("keyframe bodies are not motion declarations", () => {
  const css = `
@keyframes v2-toast-in {
  from {
    animation-timing-function: ease-out;
    opacity: 0;
  }
}
.a {
  animation: v2-toast-in var(--motion-duration-fast) var(--motion-ease-out);
}
`;
  assert.deepEqual(findStyleTokenViolations(css), []);
});

test("an exemption with a reason covers the declaration it sits on", () => {
  const css = `
.a {
  /* style-token: allow v1 skin keeps its shipped 160ms */
  transition: color 160ms ease;
}
`;
  assert.deepEqual(findStyleTokenViolations(css), []);
  assert.deepEqual(listExemptions(css), [
    { line: 3, reason: "v1 skin keeps its shipped 160ms" },
  ]);
});

test("an exemption also covers a multi-line value it starts", () => {
  const css = `
.a {
  /* style-token: allow shipped curve */
  transition: transform 240ms var(--motion-ease-out),
    opacity 240ms var(--motion-ease-out);
}
`;
  assert.deepEqual(findStyleTokenViolations(css), []);
});

test("an exemption without a reason is itself a violation", () => {
  const css = `.a {\n  /* style-token: allow */\n  transition: color 160ms ease;\n}\n`;
  const violations = findStyleTokenViolations(css);
  // The marker is reported, and it does not exempt the declaration below it:
  // one violation for the literal duration, one for the literal `ease`.
  assert.deepEqual(lines(violations), [2, 3, 3]);
  assert.match(violations[0].message, /needs a reason/u);
  assert.match(violations[1].message, /literal duration/u);
  assert.match(violations[2].message, /literal timing function/u);
});

test("an exemption that exempts nothing is reported so the list cannot rot", () => {
  const css = `
.a {
  /* style-token: allow nothing uses this */
  transition: color var(--motion-duration-fast) var(--motion-ease-out);
}
`;
  const violations = findStyleTokenViolations(css);
  assert.deepEqual(lines(violations), [3]);
  assert.match(violations[0].message, /unused/u);
});

test("bare zero durations are allowed and comments cannot fake a declaration", () => {
  const css = `
.a {
  transition-delay: 0s;
  transition-duration: 0;
  transition: none;
}
/* transition: color 160ms ease; */
`;
  assert.deepEqual(findStyleTokenViolations(css), []);
});

test("the declaration scanner reports line spans and at-rule context", () => {
  const css = `.a {\n  color: red;\n}\n@media (prefers-reduced-motion: reduce) {\n  .a {\n    transition-duration: 0.01ms;\n  }\n}\n`;
  const declarations = collectDeclarations(css);
  assert.deepEqual(
    declarations.map((declaration) => ({
      property: declaration.property,
      startLine: declaration.startLine,
      inReducedMotion: declaration.inReducedMotion,
    })),
    [
      { property: "color", startLine: 2, inReducedMotion: false },
      { property: "transition-duration", startLine: 6, inReducedMotion: true },
    ],
  );
});

test("helpers keep function groups intact", () => {
  assert.deepEqual(splitTopLevel("a 1ms, cubic-bezier(0.2, 0, 0, 1)", ","), [
    "a 1ms",
    "cubic-bezier(0.2, 0, 0, 1)",
  ]);
  assert.deepEqual(tokenize("transform var(--x, 1px) cubic-bezier(0.2, 0, 0, 1)"), [
    "transform",
    "var(--x, 1px)",
    "cubic-bezier(0.2, 0, 0, 1)",
  ]);
});
