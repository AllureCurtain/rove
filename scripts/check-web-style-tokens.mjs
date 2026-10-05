#!/usr/bin/env node
/**
 * Style-token linter (design F8).
 *
 * Rejects motion values in the Web stylesheets that bypass the token layer:
 *
 *   1. `transition` / `animation` durations must come from
 *      `var(--motion-duration-*)`. In a shorthand the first `<time>` of each
 *      comma part is the duration and must be tokenized; a second one is the
 *      delay and may stay literal.
 *   2. Timing functions must come from `var(--motion-ease-*)`, or be the
 *      literal `linear` / `steps(...)`. Keywords such as `ease-out` and a
 *      literal `cubic-bezier(...)` are not allowed: the token layer owns the
 *      curve.
 *   3. Declarations inside `@media (prefers-reduced-motion: ...)` are exempt:
 *      that block exists to neutralize motion.
 *   4. A declaration may instead carry a trailing
 *      `/* style-token: allow <reason> *\/` comment on one of its own lines. An
 *      exemption without a reason, or one that exempts nothing, is itself an
 *      error so the exemption list cannot rot. Every exemption is printed, which
 *      is how it becomes visible in review.
 *
 * Deliberately not policed: spacing and px/rem sizes, colours, radii, shadows,
 * z-index, and anything else. The linter answers one question — does motion come
 * through the token layer — and stays quiet about the rest.
 *
 * Usage:
 *   node scripts/check-web-style-tokens.mjs             # scans apps/web/styles
 *   node scripts/check-web-style-tokens.mjs <path...>    # scans explicit paths
 *   node --test scripts/check-web-style-tokens.test.mjs  # self-tests
 *
 * Exit code 0 means the stylesheets are clean and every exemption is documented;
 * 1 means at least one violation, printed as `file:line: message`.
 */

import { readFileSync, statSync } from "node:fs";
import { readdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const DEFAULT_TARGETS = ["apps/web/styles"];

const DURATION_TOKEN = /^var\(\s*--motion-duration-[a-z0-9-]+\s*(?:,[^)]*)?\)$/u;
const EASE_TOKEN = /^var\(\s*--motion-ease-[a-z0-9-]+\s*(?:,[^)]*)?\)$/u;
const ALLOWED_EASE = /^(?:linear|step-start|step-end|steps\(.*\)|linear\(.*\))$/u;
const TIME_VALUE = /^-?(?:\d+\.?\d*|\.\d+)(?:ms|s)$/u;
const ZERO_TIME = /^-?0(?:ms|s)?$/u;
const EASE_KEYWORD = /^(?:ease|ease-in|ease-out|ease-in-out)$/u;
const EASE_FUNCTION = /^(?:cubic-bezier|steps|linear)\(/u;
const EXEMPTION = /\/\*\s*style-token:\s*allow\b([^*]*)\*\//u;
const EXEMPTION_HEAD = /\/\*\s*style-token:\s*allow\b/u;

const DURATION_PROPERTIES = new Set(["transition-duration", "animation-duration"]);
const TIMING_PROPERTIES = new Set([
  "transition-timing-function",
  "animation-timing-function",
]);
const SHORTHANDS = new Set(["transition", "animation"]);

/** Split on a top-level separator, keeping `fn(...)` groups intact. */
export function splitTopLevel(text, separator) {
  const parts = [];
  let depth = 0;
  let current = "";
  for (const char of text) {
    if (char === "(") {
      depth += 1;
    } else if (char === ")") {
      depth -= 1;
    }
    if (char === separator && depth === 0) {
      parts.push(current);
      current = "";
      continue;
    }
    current += char;
  }
  parts.push(current);
  return parts.map((part) => part.trim()).filter((part) => part.length > 0);
}

/** Whitespace tokens with `fn(...)` groups kept whole. */
export function tokenize(value) {
  const tokens = [];
  let depth = 0;
  let current = "";
  for (const char of value) {
    if (char === "(") {
      depth += 1;
    } else if (char === ")") {
      depth -= 1;
    }
    if (/\s/u.test(char) && depth === 0) {
      if (current.length > 0) {
        tokens.push(current);
      }
      current = "";
      continue;
    }
    current += char;
  }
  if (current.length > 0) {
    tokens.push(current);
  }
  return tokens;
}

/** Replace comment bodies with spaces so they can never parse as declarations. */
function maskComments(text) {
  return text.replace(/\/\*[\s\S]*?\*\//gu, (match) =>
    match.replace(/[^\n]/gu, " "),
  );
}

/**
 * Flatten a stylesheet into declarations with their 1-based line spans and
 * at-rule context. The scan is forward-only and single-pass: brace depth, and
 * therefore the context of every declaration, is known when it is emitted.
 */
export function collectDeclarations(cssText) {
  const masked = maskComments(cssText);
  const declarations = [];
  const contexts = [];
  const lineStarts = [0];
  for (let index = 0; index < cssText.length; index += 1) {
    if (cssText[index] === "\n") {
      lineStarts.push(index + 1);
    }
  }
  const lineAt = (offset) => {
    let low = 0;
    let high = lineStarts.length - 1;
    while (low < high) {
      const middle = Math.ceil((low + high) / 2);
      if (lineStarts[middle] <= offset) {
        low = middle;
      } else {
        high = middle - 1;
      }
    }
    return low + 1;
  };

  let buffer = "";
  let start = -1;

  const emit = (endOffset) => {
    const raw = buffer.trim();
    const startOffset = start;
    buffer = "";
    start = -1;
    if (raw.length === 0) {
      return;
    }
    const colon = raw.indexOf(":");
    if (colon === -1) {
      return;
    }
    declarations.push({
      property: raw.slice(0, colon).trim().toLowerCase(),
      value: raw.slice(colon + 1).trim(),
      startLine: lineAt(startOffset),
      endLine: lineAt(endOffset),
      inReducedMotion: contexts.includes("reduced-motion"),
      inKeyframes: contexts.includes("keyframes"),
    });
  };

  const openContext = (prelude) => {
    const text = prelude.trim().toLowerCase();
    if (text.startsWith("@media") && text.includes("prefers-reduced-motion")) {
      contexts.push("reduced-motion");
    } else if (/(^|\s)@(-\w+-)?keyframes(\s|$)/u.test(` ${text} `)) {
      contexts.push("keyframes");
    } else {
      contexts.push("block");
    }
  };

  for (let index = 0; index < masked.length; index += 1) {
    const char = masked[index];
    if (char === "{") {
      openContext(buffer);
      buffer = "";
      start = -1;
    } else if (char === "}") {
      emit(index);
      contexts.pop();
    } else if (char === ";") {
      emit(index);
    } else {
      if (start === -1 && !/\s/u.test(char)) {
        start = index;
      }
      buffer += char;
    }
  }
  emit(masked.length);

  return declarations;
}

/**
 * An exemption may sit on any line of the declaration (a trailing comment, or a
 * line inside a multi-line value) or on the line directly above it.
 */
function exemptionFor(declaration, exemptionLines) {
  const candidates = [declaration.startLine - 1];
  for (let line = declaration.startLine; line <= declaration.endLine; line += 1) {
    candidates.push(line);
  }
  return candidates.find((line) => (exemptionLines.get(line) ?? "").length > 0);
}

/**
 * Find every style-token violation in one stylesheet.
 *
 * @param {string} cssText
 * @param {string} [fileName] name used in messages
 * @returns {{file: string, line: number, message: string}[]}
 */
export function findStyleTokenViolations(cssText, fileName = "style.css") {
  const violations = [];
  const declarations = collectDeclarations(cssText);
  const lines = cssText.split("\n");

  const exemptionLines = new Map();
  lines.forEach((text, index) => {
    const match = EXEMPTION.exec(text);
    if (match) {
      const reason = (match[1] ?? "").trim();
      exemptionLines.set(index + 1, reason);
      if (reason.length === 0) {
        violations.push({
          file: fileName,
          line: index + 1,
          message:
            "style-token exemption needs a reason: `/* style-token: allow <reason> */`",
        });
      }
    } else if (EXEMPTION_HEAD.test(text)) {
      violations.push({
        file: fileName,
        line: index + 1,
        message:
          "style-token exemption needs a reason: `/* style-token: allow <reason> */`",
      });
    }
  });

  const used = new Set();
  const reported = new Set();
  const record = (declaration, message) => {
    const exemption = exemptionFor(declaration, exemptionLines);
    if (exemption !== undefined) {
      used.add(exemption);
      return;
    }
    const key = `${declaration.startLine}:${message}`;
    if (reported.has(key)) {
      return;
    }
    reported.add(key);
    violations.push({ file: fileName, line: declaration.startLine, message });
  };

  for (const declaration of declarations) {
    if (declaration.inReducedMotion || declaration.inKeyframes) {
      continue;
    }
    const parts = splitTopLevel(declaration.value, ",");

    if (DURATION_PROPERTIES.has(declaration.property)) {
      for (const token of tokenize(declaration.value)) {
        if (ZERO_TIME.test(token)) {
          continue;
        }
        if (TIME_VALUE.test(token) && !DURATION_TOKEN.test(token)) {
          record(
            declaration,
            `${declaration.property}: \`${token}\` is a literal duration; use var(--motion-duration-*)`,
          );
        }
      }
    }

    if (TIMING_PROPERTIES.has(declaration.property)) {
      for (const part of parts) {
        for (const token of tokenize(part)) {
          if (EASE_TOKEN.test(token) || ALLOWED_EASE.test(token)) {
            continue;
          }
          if (EASE_KEYWORD.test(token) || EASE_FUNCTION.test(token)) {
            record(
              declaration,
              `${declaration.property}: \`${token}\` is a literal timing function; use var(--motion-ease-*) (or linear/steps())`,
            );
          }
        }
      }
    }

    if (SHORTHANDS.has(declaration.property)) {
      for (const part of parts) {
        const tokens = tokenize(part);
        // The first duration slot of a part is the duration; a later literal
        // time is the delay. A tokenized duration claims that slot.
        let sawDuration = false;
        for (const token of tokens) {
          if (DURATION_TOKEN.test(token)) {
            sawDuration = true;
            continue;
          }
          if (TIME_VALUE.test(token) && !ZERO_TIME.test(token)) {
            if (!sawDuration) {
              sawDuration = true;
              record(
                declaration,
                `${declaration.property}: \`${token}\` is a literal duration; use var(--motion-duration-*)`,
              );
            }
            continue;
          }
          if (EASE_TOKEN.test(token) || ALLOWED_EASE.test(token)) {
            continue;
          }
          if (EASE_KEYWORD.test(token) || EASE_FUNCTION.test(token)) {
            record(
              declaration,
              `${declaration.property}: \`${token}\` is a literal timing function; use var(--motion-ease-*) (or linear/steps())`,
            );
          }
        }
      }
    }
  }

  for (const [line, reason] of exemptionLines) {
    if (reason.length === 0 || used.has(line)) {
      continue;
    }
    violations.push({
      file: fileName,
      line,
      message: `style-token exemption is unused; remove it or its literal is already allowed (reason: ${reason})`,
    });
  }

  return violations.sort((left, right) => left.line - right.line);
}

/** Every documented exemption in a stylesheet, for the printed inventory. */
export function listExemptions(cssText) {
  return cssText
    .split("\n")
    .map((text, index) => {
      const match = EXEMPTION.exec(text);
      const reason = (match?.[1] ?? "").trim();
      return match && reason.length > 0 ? { line: index + 1, reason } : null;
    })
    .filter(Boolean);
}

export function formatViolations(violations) {
  return violations
    .map((violation) => `${violation.file}:${violation.line}: ${violation.message}`)
    .join("\n");
}

async function collectCssFiles(targets) {
  const files = [];
  for (const target of targets) {
    const absolute = path.resolve(REPO_ROOT, target);
    const stats = statSync(absolute);
    if (stats.isDirectory()) {
      const entries = await readdir(absolute, { withFileTypes: true });
      for (const entry of entries.sort((left, right) => left.name.localeCompare(right.name))) {
        const child = path.join(target, entry.name);
        if (entry.isDirectory()) {
          files.push(...(await collectCssFiles([child])));
        } else if (entry.name.endsWith(".css")) {
          files.push(child.split(path.sep).join("/"));
        }
      }
    } else if (target.endsWith(".css")) {
      files.push(target.split(path.sep).join("/"));
    }
  }
  return files;
}

async function main() {
  const targets = process.argv.slice(2).filter((argument) => !argument.startsWith("-"));
  const files = await collectCssFiles(targets.length > 0 ? targets : DEFAULT_TARGETS);
  const violations = [];
  let exemptionCount = 0;

  for (const file of files) {
    const cssText = readFileSync(path.resolve(REPO_ROOT, file), "utf8");
    violations.push(...findStyleTokenViolations(cssText, file));
    for (const exemption of listExemptions(cssText)) {
      exemptionCount += 1;
      console.log(`exempt ${file}:${exemption.line}: ${exemption.reason}`);
    }
  }

  if (violations.length > 0) {
    console.error(formatViolations(violations));
    console.error(
      `\n${violations.length} style-token violation(s) across ${files.length} stylesheet(s).`,
    );
    process.exitCode = 1;
    return;
  }

  console.log(
    `style tokens ok: ${files.length} stylesheet(s), ${exemptionCount} documented exemption(s).`,
  );
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main();
}
