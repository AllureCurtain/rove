# rove Web

Next.js 16 + React 19 product UI, talking to `rove-api` over REST + SSE. The main surface is the product shell (workspace → session → run); `/dev/workbench` is a development-oriented advanced entry.

## Layout

```
app/          routes: / product shell, /dev/workbench advanced entry, /api/* same-origin proxy
shell/        shell layout: top bar, panels, drawers, skins and themes
sidebar/      workspace and session navigation
chat/         transcript, Composer, message delivery state
inspector/    right-hand inspection panel (files, diffs, artifacts, usage, review)
settings/     settings sections
search/       search surfaces
components/   shared components
state/        client state: product catalog, event streams, drafts, providers, transcript projection
product/      product API client (product-client.ts) and hand-written API types
lib/          rove API client, /api proxy, shared utilities
api/          run controllers
platform/     Desktop/Tauri adaptation
copy/         Chinese and English UI copy
styles/       three-layer styles and tokens; rules in the root DESIGN.md
tests/e2e/    Playwright
```

Conventions in `AGENTS.md`: state uses React capabilities plus stores under `state/`; API calls go through `product-client.ts` / `rove-client.ts`; the browser only requests same-origin `/api/*`, forwarded by a server-side proxy that injects `ROVE_API_TOKEN` — raw provider keys never reach the browser.

## Development

```powershell
pnpm install --frozen-lockfile
pnpm dev        # needs rove-api; scripts/dev.ps1 starts API + Web together
```

Open <http://localhost:3000>. The proxy forwards to `http://127.0.0.1:8787` by default (`ROVE_API_BASE` overrides; `ROVE_API_TOKEN` supplies auth).

## Verification

```powershell
pnpm test               # vitest unit tests, colocated with sources
pnpm typecheck
pnpm build
pnpm test:e2e           # Playwright, mocked backend
pnpm test:e2e:prod      # against the production build
pnpm build:desktop      # Desktop static bundle (apps/web/desktop-dist)
pnpm lint:style-tokens  # style-token check, local gate
```

`real-api.spec.ts` only runs against a real `rove-api` under the `local-full` integration profile; see `docs/development.md`.
