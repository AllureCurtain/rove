# Web conventions

Global rules live in the root AGENTS.md; this file only covers `apps/web` specifics.

## Layout

```
app/          Next.js routes; / is the product shell, /dev/workbench is the advanced entry
shell/        shell layout: top bar, panels, drawers
sidebar/      workspace and session navigation
chat/         transcript, Composer, message delivery state, transcript windowing
inspector/    right-hand inspection panel (files, diffs, artifacts, usage)
settings/     settings sections
search/       search surfaces
components/   shared components
state/        client state: product catalog, event streams, drafts, providers, transcript projection
product/      product API client (product-client.ts) and generated-facing API types (product-api-types.ts aliases `generated/api-types.ts`)
lib/          rove API client, /api proxy, shared utilities
api/          run controllers
platform/     Desktop/Tauri adaptation
copy/         UI copy
styles/       three-layer styles and tokens; see DESIGN.md
tests/e2e/    Playwright
```

## Conventions

- State management uses only React's built-in capabilities plus stores under `state/`; no Redux, Zustand, TanStack, or similar. Server data is authoritative via the API; the client only projects and caches.
- API calls go through `product/product-client.ts` and `lib/rove-client.ts`; components never call `fetch` directly.
- The browser only requests same-origin `/api/*`. Under `next dev`/`next start` that is forwarded server-side by `lib/rove-api-proxy.ts` with `ROVE_API_TOKEN` injected; in the `pnpm build:web` bundle served by `rove-api --web-dist` it resolves directly against the API mounted at `/api` on the same origin. Raw provider keys never enter browser state, `localStorage`, or logs — with exactly one exception: `POST /product/provider-onboarding` carries the pasted credential transiently (uncontrolled input, read once at submit), and the API only accepts it on a loopback bind (see docs/api.md).
- API types are generated from `apps/api/openapi.json` (`pnpm gen:api-types`; `pnpm check:api-types` fails on drift). When the API changes, the Rust side and `utoipa` annotations change first, then `pnpm gen:api-types` regenerates and call sites are fixed; `product/product-api-types.ts` keeps boundary guards and request validators, not response schemas (see docs/api.md).
- Real-time data goes over SSE with `Last-Event-ID` resume; prefer push over polling wherever possible.
- A product session has at most one active turn; when a task-start response is ambiguous, perform a bounded reconciliation first — never auto-resubmit.
- Styles use only the tokens in DESIGN.md; motion durations and easings are enforced by `pnpm lint:style-tokens`.
- Unit tests (`*.test.ts(x)`) live next to their sources. e2e uses `tests/e2e/product-api-mock.ts` by default; `real-api.spec.ts` only runs against a real API under `local-full`.
- The Desktop static bundle is produced by `pnpm build:desktop`; a separate Desktop-only UI is not allowed.
