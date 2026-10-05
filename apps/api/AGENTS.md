# rove-api conventions

Global rules live in the root AGENTS.md; this file only covers `apps/api` specifics. General API conventions live in docs/api.md.

## Layers

```
src/lib.rs                 shared type declarations (ApiState, JobRecord, etc.), router, serve*
src/state.rs               ApiState construction and accessors
src/jobs.rs                /jobs, /runs, /providers handlers, generic job start
src/launch.rs              product turn launch, product resume-state loading
src/followup.rs            successor messages: start, requeue/retry, drain, steer replay
src/fork.rs                fork boundary validation and source-state projection
src/supervisor.rs          job supervisor: consumes the event stream, reflects control events, terminal cleanup
src/review.rs              product Review runtime
src/assembly.rs            Engine assembly, workspace and config resolution, background maintenance tasks
src/events.rs              job state and SSE projection, approval/input providers
src/error.rs               ApiError and HTTP response mapping
src/tests.rs               lib-level unit tests
src/security.rs            bearer auth, CORS, rate-limit middleware
src/types.rs               job and run request/response types, ApiErrorResponse
src/product/routes.rs      /product/* routes
src/product/*.rs           the HTTP side of each product domain: attachment bytes, artifacts, diffs, files, MCP, memory, Review, trust, migration, transcripts
../product-store/          rove-product-store: /product/* contract types and limit constants (contracts.rs), ProductStore (store/schema.rs migrations, store/repository.rs data access)
src/benchmark.rs           /bench/* routes (depends on rove-bench)
```

## Conventions

- Route handlers only validate parameters and orchestrate; business logic goes in `product/*` or runtime, and data access goes only through `rove-product-store`. Contract types are added in `rove-product-store`; never define a second copy inside api.
- New or changed routes must carry `#[utoipa::path]`, derive `ToSchema`, and register with the OpenApi router. After API changes, regenerate the `apps/api/openapi.json` snapshot with `ROVE_UPDATE_OPENAPI=1 cargo test -p rove-integration-tests --test api openapi_snapshot`.
- Errors uniformly return `ApiError` as `ApiErrorResponse { code, error }`. Outward-facing error codes use explicitly named `*_with_code` constructors; error messages are sanitized before returning.
- JSON request bodies are received as `Result<Json<T>, JsonRejection>` and produce fixed-text errors — serde error details are never echoed back.
- Every input has a limit: request body size, list `limit`, file read windows. Names, URIs, and MIME types from remote sources are never concatenated into local paths.
- All SQL is parameterized.
- ProductStore schema may only change via versioned migrations in `apps/product-store/src/store/schema.rs`: bump `CURRENT_SCHEMA_VERSION` by 1, migrations must be idempotent and upgradeable from any older version. Refuse to open a database newer than the current version.
- ProductStore holds product control state only and never duplicates canonical events. Lifecycle facts worth recording go through the canonical `StreamEvent`.
- Write endpoints: revisioned resources use `expected_revision` for CAS; side-effecting creates support `idempotency_key`.
- Every new endpoint gets at least one test: a module unit test or an entry in `tests/api.rs`.
