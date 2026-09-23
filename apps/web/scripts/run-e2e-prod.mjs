import { spawn } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/**
 * Production-build end-to-end profile.
 *
 * `pnpm test:e2e` runs the suite against `next dev`. Two behaviours are only observable in
 * a production build: route payload prefetching (`route-prefetch.spec.ts`), and a correct
 * status for the `/dev/*` preview routes, which `app/dev/layout.tsx` resolves to a 404
 * unless `ROVE_ENABLE_DEV_ROUTES` is set **while the build runs** (the pages prerender).
 *
 * This runner therefore builds and then tests with both gates in the environment, and
 * `playwright.config.ts` starts `next start` for `ROVE_E2E_PROD=1`. Extra CLI arguments are
 * forwarded to `playwright test`, e.g.:
 *
 *   pnpm test:e2e:prod tests/e2e/route-prefetch.spec.ts
 */
const webRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const nextBin = join(webRoot, "node_modules", "next", "dist", "bin", "next");
const playwrightBin = join(webRoot, "node_modules", "@playwright", "test", "cli.js");

const env = {
  ...process.env,
  ROVE_E2E_PROD: "1",
  ROVE_ENABLE_DEV_ROUTES: "1",
};

function run(args) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, args, {
      cwd: webRoot,
      env,
      stdio: "inherit",
    });
    child.on("error", reject);
    child.on("exit", (code, signal) => {
      if (code === 0) {
        resolve();
        return;
      }
      reject(new Error(`${args.join(" ")} exited with ${signal ?? code}`));
    });
  });
}

await run([nextBin, "build"]);
await run([playwrightBin, "test", ...process.argv.slice(2)]);
