import { cp, mkdir, readFile, rm, stat } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const webRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const nextRoot = join(webRoot, ".next");
const outputRoot = join(webRoot, "desktop-dist");
const sourceHtml = join(nextRoot, "server", "app", "index.html");

await stat(sourceHtml);
await rm(outputRoot, { recursive: true, force: true });
await mkdir(join(outputRoot, "_next"), { recursive: true });
await cp(sourceHtml, join(outputRoot, "index.html"));
await cp(join(nextRoot, "static"), join(outputRoot, "_next", "static"), {
  recursive: true,
});

const publicRoot = join(webRoot, "public");
try {
  await stat(publicRoot);
  await cp(publicRoot, outputRoot, { recursive: true });
} catch (error) {
  if (error?.code !== "ENOENT") throw error;
}

const html = await readFile(join(outputRoot, "index.html"), "utf8");
if (!html.includes("/_next/static/")) {
  throw new Error("desktop HTML does not reference the expected Next static assets");
}
