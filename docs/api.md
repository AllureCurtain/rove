# API conventions

Per-endpoint fields are authoritative in the runtime-generated OpenAPI: `GET /api/openapi.json`, Swagger UI at `/swagger-ui`. The spec is generated from utoipa annotations in `apps/api/src` (`#[utoipa::path]`, `ToSchema`); the checked-in `apps/api/openapi.json` is its snapshot, and a consistency test in `tests/api.rs` keeps them from drifting (regenerate after API changes with `ROVE_UPDATE_OPENAPI=1 cargo test -p rove-integration-tests --test api openapi_snapshot`). This file only covers general conventions.

## Basics

- Address: `http://127.0.0.1:8787` by default; Desktop uses a random loopback port.
- Route groups, with no `/api/v1`-style version prefix:
  - `/jobs`, `/runs`: runtime jobs and runs
  - `/product/*`: the product control plane — workspaces, sessions, preferences, providers, MCP, memory, Review, search, migration, events
  - `/providers/models`, `/providers/test`: provider probing
  - `/bench/*`: benchmarks
  - `/debug/*`: debugging
- `GET /health`: liveness for launch scripts (`{"status":"ok","version":...}`), outside the OpenAPI surface like Swagger UI.
- With `--web-dist` (or `ROVE_WEB_DIST`) the same process additionally serves the built Web console at the origin root and mounts the full API a second time under `/api` — the bundle's clients only ever call same-origin `/api/*`. Root paths keep working for existing clients (Desktop transport, `scripts/dev.ps1`, Swagger UI). Unknown paths fall back to `index.html` for client-side routing. Static assets and `/health` are public; the API keeps its auth.
- Auth: with `api.token_auth` configured, business routes require `Authorization: Bearer <token>`, corresponding to `BearerAuth` in OpenAPI. Documentation endpoints need no auth.
- The Web browser never carries a token directly: in dev it goes through the Next.js server-side `/api/*` proxy, which injects `ROVE_API_TOKEN`; under `--web-dist` serving, the bundle calls same-origin `/api/*` directly — that mode is meant for the zero-config loopback case (no `token_auth`), and a browser credential hand-off for the token-configured case is a separate decision.
- CORS only allows origins in `ROVE_API_CORS_ORIGINS`; an `Origin` whose authority equals the request's `Host` is treated as same-origin, not cross-origin, and needs no allowlist entry. Rate limiting is per-process (`ROVE_API_RATE_LIMIT_PER_MINUTE`), not distributed.
- `POST /product/provider-onboarding` is the only route whose body carries a raw provider credential (`credential`, `write_only` in the spec). It is refused with 403 `provider_onboarding_loopback_required` unless `api.bind_addr` is a loopback address: a remotely reachable socket must never accept secrets for this machine's keyring. The credential is registered for redaction, handed to the OS credential store by the shared `ProviderOnboardingService`, and zeroized on return — it never reaches ProductStore, a response body, or logs. Bounded fields, `deny_unknown_fields`, and fixed parse errors apply as usual. Browser clients should only offer this path from a loopback-served page; Desktop continues to use the native credential prompt.
- Request bodies have a size cap; migration and attachment uploads have separate larger caps (`MAX_*_BODY_BYTES` in `apps/product-store/src/contracts.rs`).
- Time: RFC 3339 / ISO 8601, UTC.

## Response format

Success returns the resource JSON directly, with no envelope.

Failure (`ApiErrorResponse`, `apps/api/src/types.rs`):

```json
{ "code": "conflict", "error": "safe message for the user" }
```

- `error` is sanitized before the response is built.
- A failed JSON body parse returns fixed text plus the route's own error code (`crate::json_body`); raw values from serde errors are never echoed.

## Error codes

| code | HTTP | meaning |
| --- | --- | --- |
| `bad_request` | 400 | parameter validation failed |
| `not_found` | 404 | resource does not exist |
| `conflict` | 409 | revision conflict, active turn already held, etc. |
| `bad_gateway` | 502 | upstream provider or MCP failure |
| `internal_error` | 500 | internal error |
| domain codes | 400/403/404/409/502/503/504 | built via `*_with_code`, e.g. `provider_onboarding_required`, `provider_changed_for_resume`, `state_migration_conflict` |

Clients should branch on `code` and never parse the `error` text.

## Pagination

- Lists use opaque cursors: the request carries `cursor` and `limit`; the response returns `next_cursor`, empty when there is no next page.
- Transcript back-pagination: the request carries `before_ordinal` and `limit_runs`; the response returns `next_before_ordinal` and `has_more`.
- The server truncates `limit` to a maximum.

## Concurrency and idempotency

- Writes to revisioned resources (preferences, provider catalog, etc.) must carry `expected_revision`; a mismatch returns 409. The provider catalog's version field is `catalog_revision`.
- Side-effecting creates (send message, fork, migrate, etc.) carry `idempotency_key`; retries return the same result.
- `POST /jobs` allows only one active turn per product session.

## Streaming (SSE)

- `GET /jobs/{job_id}/events`: replays persisted events first, then streams live events; replay still works after a server restart.
- `GET /product/events`: the product journal, capped at the latest 10000 lines. Resume with SSE `id:` + `Last-Event-ID`; no polling needed.
- Event payloads are the canonical `StreamEvent`, sanitized once more before sending.
- Clients declare `event_contract` in the request to select a transcript event contract version. Without it, the old contract applies and bytes are unchanged; the server never infers the contract from UA or other fields.

## Change process

1. Update types and utoipa annotations in `apps/api/src` (new fields need defaults and compatibility).
2. Update the backend implementation and add tests (`apps/api` unit tests, or `tests/api.rs`).
3. Sync the hand-written frontend types (`apps/web/product/product-api-types.ts`, `apps/web/lib/rove-types.ts`) and call sites.
4. For event changes: the producer, persistence, SSE, Web consumers, and the contract test (`tests/event_contract.rs`) all change together.
