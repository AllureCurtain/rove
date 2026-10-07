# Technical decisions

New decisions go on top. Do not delete overturned decisions; mark them "superseded" and note which entry replaced them.

Decisions before 2026-10-01 were distilled from the design documents of that period; the originals have been deleted.

## 2026-10-07 Web API types generated from OpenAPI; SSE event schemas registered first

- Status: active
- Decision: the hand-written REST types in `apps/web/product/product-api-types.ts` are replaced by `openapi-typescript` output generated from the checked-in `apps/api/openapi.json`; a small hand-written validation layer stays at trust boundaries (responses are still runtime-checked where a wrong shape would corrupt state). SSE event payloads are out of scope for `openapi.json` until their types carry `ToSchema`: `StreamEvent` and its payload graph get `utoipa::ToSchema` derives and register in the `ApiDoc` components so `event` stops being a bare `object`. The hand-maintained e2e mock `tests/e2e/product-api-mock.ts` is then type-checked against the generated types instead of the removed hand-written ones.
- Why: the API's utoipa annotations are already authoritative; hand-written TypeScript mirrors drift silently (there is a snapshot test for the spec itself, but nothing guards the TS side). Generating removes the drift class; registering event schemas in OpenAPI is what makes the generated types useful for streams rather than only request/response envelopes.
- Rejected: schemars side-car schema export for `StreamEvent` — produces a second schema pipeline that can disagree with utoipa; keeping runtime validators everywhere — overkill once types are generated, validators stay only where a malformed payload is a real risk.

## 2026-10-07 Loopback-only provider credential entry over HTTP

- Status: active
- Decision: `POST /product/provider-onboarding` is the single HTTP route that accepts a raw provider key (`credential`, marked `write_only` in OpenAPI). It is refused with `provider_onboarding_loopback_required` unless `api.bind_addr` parses to a loopback address — checking the configured bind, not the peer, because a loopback listener is unreachable remotely while a non-loopback one cannot distinguish callers. The body is bounded, `deny_unknown_fields`, and parsed through the fixed-error path; the secret is registered for redaction before onboarding, handed to the OS keyring via the shared `ProviderOnboardingService`, and wrapped in `zeroize::Zeroizing`. The receipt is secret-free. On the Web side the field is an uncontrolled input read once at submit, offered only when the page origin is itself loopback; Desktop keeps its native credential prompt. Stage two (token-authenticated browser hand-off: one-time URL, HttpOnly cookie, or login page) is deliberately undecided — with `api.token_auth` on, a browser that can authenticate could already use this route, so the hand-off remains a separate reviewed decision.
- Why: paste-key onboarding existed only in Desktop (in-process Tauri command), leaving the browser control surface unable to configure a real provider without touching the CLI — a hard gap for "web-first" usage. Reusing the shared onboarding service keeps credential storage, probing, catalog CAS, and compensation on one implementation.
- Rejected: letting the route trust the remote peer address — a `0.0.0.0` bind sees remote clients too; a dedicated "local mode" token or per-request handshake — the bind check already expresses the threat model and adds no moving parts; storing the key in ProductStore or returning a reference to it — keyring-only was already the established contract.

## 2026-10-07 rove-api hosts the Web bundle on one origin; the API answers under `/api` too

- Status: active
- Decision: `rove-api --web-dist <dir>` (or `ROVE_WEB_DIST`) serves the `pnpm build:web` static bundle (`apps/web/web-dist`) at the origin root with an `index.html` SPA fallback, and mounts the same router a second time under `/api` so the bundle's same-origin `/api/*` calls resolve without the Next.js proxy. `router()` stays API-only; `GET /health` and statics sit outside the security middleware, while both API mounts keep bearer auth. An `Origin` matching the request's `Host` is treated as same-origin rather than a CORS candidate. `scripts/serve.ps1` is the single-command product launcher (build + run); token-authenticated browser access is intentionally left for a separate credential hand-off decision, and startup warns when `--web-dist` and `api.token_auth` are combined.
- Why: the external control surface had no product-shaped launch — it required two processes (`rove-api` + `next dev`) with a server-side proxy just to inject a token. One process serving both halves is the smallest change that makes the console independently runnable; the existing desktop bundle build is reused verbatim rather than inventing a second packaging path.
- Rejected: `output: export` static export — `app/api/[...path]` is a route handler and the build cannot be fully static without deleting it; an `if path.starts_with("/api")` strip inside the security layer — mounting under `nest("/api")` is explicit routing, not path rewriting; a `rove serve` subcommand — `rove-api` is already the right binary and duplicating its arg surface in rove-cli adds a second launch path.

## 2026-10-06 P4 polish: TodoDock on `runState.plan`, proportional minimap, stacking ladder

- Status: active
- Decision: the TodoDock composer fold reads `runState.plan` directly — the reducer already maintains `TaskPlan` from `plan_created`/`step_result` restore and live events, so no parallel plan store exists. The conversation minimap maps each rendered turn to its proportional document position (via `alignMinimapMarkers`, which clamps and spreads dense clusters to a minimum pitch) and draws a viewport band driven by the transcript scroller, replacing the uniform spacing that drifted from the real positions. Stacking is now a named ladder: `--z-inset…--z-modal` on `.product-app-frame` replaces 28 literal `z-index` values, and elevation derives from a per-skin `--cp-elev-ch` channel (warm ink `60 40 20`, graphite/dark near-black `0 0 0`) consumed by `--cp-shadow-*` and `rgb(var(--cp-elev-ch) / <alpha>)` washes. The transcript log exposes `data-testid="conversation-log"` so e2e selectors no longer depend on a hardcoded English `aria-label`. The stale `--sidebar-width`/`--inspector-width` redefinitions under the v2 scope were removed (the rail runs on `--sidebar-nav-width`, the panel on `--work-panel-resolved`), which is what the DESIGN.md 240/248 confusion traced to.
- Why: design §6 puts the plan fold in the composer stack and §2.11 requires the minimap to align with the reading band; category B asks for a semantic ladder and same-hue alpha elevation.
- Rejected: a separate plan store keyed by session — duplicates the reducer contract and can diverge on restore.

## 2026-10-06 Work panel: per-session id-based tab strip in localStorage

- Status: active
- Decision: the work-panel tab strip is a dynamic set identified by id (`file:<path>` for per-file viewer tabs, the kind otherwise), persisted per `workspace::session` pair in `localStorage` under `rove.ui-work-panel-tabs` (bounded to 24 sessions) alongside the existing width (`rove.ui-work-panel-width`) and open (`rove.ui-work-panel-open`) preferences. Maximize is deliberately a session-only mode and is never persisted. Two input details are load-bearing: pointer capture for tab drag-reorder is deferred until the press crosses the drag threshold — capturing on `pointerdown` retargets the click to the wrapper and silently swallows activation; and middle-click close rides on `pointerup` (`button === 1`) rather than `auxclick`, because a scrollable ancestor lets middle-click latch autoscroll, which suppresses `auxclick` entirely on Windows Chromium. Evidence export moved from the run tab to the session row menu (`onExportSession` → a modal hosting `ExportPanel`), keeping the panel reserved for per-run evidence.
- Why: design §8 requires reopening a session to restore exactly the strip the user left, and per-file tabs cannot be expressed by a kind-keyed model; the two event-handling choices are the only forms verified to work across platforms (the deferred capture was measured swallowing `click`; `auxclick` never dispatched under autoscroll).
- Rejected: persisting maximize — it is a browsing mode, not a preference; `onAuxClick` alone — works on Linux but is silently absent on Windows when any ancestor can scroll; capture-on-pointerdown — breaks plain click activation.

## 2026-10-05 Web rail: local order/pin preferences, no session-to-workspace moves

- Status: active
- Decision: session pinning and project ordering in the session rail are per-machine UI preferences held in `localStorage` (`rove.ui-pinned-sessions`, `rove.ui-rail-project-order`), like the rail/panel width and reading-width preferences. Workspace rename reuses the `createWorkspace` upsert keyed by canonical root; reveal-in-folder goes through the existing `show_in_folder` desktop command. Sessions are never reassigned between workspaces: the update contract has no `workspace_id`, and a session's runtime binding is scoped to its workspace root, so dragging a session onto a project is not offered.
- Why: the product contract carries no ordering/pinning fields, and the design's ownership table assigns these to UI preferences rather than durable product state; inventing a schema field for them would widen the contract for a view concern.
- Rejected: adding `sort_order`/`pinned` columns to ProductStore — durable schema for a per-machine view preference; session move — the contract does not support it and faking it as copy+delete would orphan runtime bindings.

## 2026-10-05 Shared `target/` for worktrees via root `.cargo/config.toml`

- Status: active
- Decision: the repository root carries an *untracked* `.cargo/config.toml` with `build.target-dir = "target"` (the path resolves relative to `rove/`, the config's parent). Cargo's ancestor config search reaches it from any worktree under `.worktrees/`, so every checkout compiles into the single root `target/` directory. The file must stay untracked: a committed copy would also exist inside each worktree at `.worktrees/<topic>/.cargo/config.toml`, where the same relative value resolves to a per-worktree `target/` and — being the closer config — wins, silently undoing the sharing.
- Why: a full workspace `target/` is multi-GB; one per worktree duplicates it per task. Sharing one directory is safe because Cargo takes a lock on concurrent access.
- Rejected: committing the config — relative `target-dir` resolves per containing directory and cannot describe both depths; `CARGO_TARGET_DIR` per command — works but is invisible and easy to forget.

## 2026-10-04 CI: cache target, desktop moved to paths-gated trigger

- Status: active
- Decision: CI uses `Swatinem/rust-cache` to cache both the cargo registry and `target/` (including cache-on-failure); routine `cargo clippy`/`cargo test --workspace` exclude `rove-desktop`; desktop runs clippy + test separately in `.github/workflows/desktop.yml` when `apps/desktop`, the root `Cargo.toml`/`Cargo.lock`, or the workflow itself changes.
- Why: previously every run compiled about 700 dependencies plus the whole workspace from scratch, and the Tauri/WebKit dependency tree accounts for the bulk of compile time while rarely changing. Why not reuse `actions/cache`: it only handles static paths, does not prune stale entries, and does not scope increments per job; `rust-cache` is the standard for Rust projects (a curated third-party action in the same class as `dtolnay/rust-toolchain`, already in use).
- Rejected: adding `paths-ignore` for doc-only changes — `tests/code_hygiene.rs` reads repo files (`.gitignore`, benchmark results README) and asserts on them, so doc changes can break tests; removing desktop from CI entirely — the paths filter is narrowed to desktop's own files plus shared manifests, and the residual risk (a runtime change breaking the desktop build) is covered by release-gate and local builds.

## 2026-10-04 Remove the v1 skin runtime switch

- Status: active
- Decision: removed the `ROVE_PRODUCT_UI_VERSION` env switch, the `ProductUiVersion`/`uiVersion` properties, and the never-consumed `data-presentation` attribute; v2 is the only product presentation version and `data-ui-version="v2"` remains the style-scoping contract. `product.css` stays as the base layer (reset and shared component rules) and no longer carries a switchable v1 presentation.
- Why: the v1 fallback has been unused since the warm skin shipped; removing the switch leaves a single presentation path.
- Rejected: keeping v1 as an escape hatch; also deleting `product.css` — it is the base layer for v2 styles and removing it would break the current presentation.

## 2026-10-03 Publicly visible content must not name references or contain local paths

- Status: active
- Decision: everything committed to the repo — code comments, docs, tests, commit messages, PR descriptions — must not contain external project names, their internal file paths, or their item numbers, and must not contain developer machine absolute paths (directory structures, drive letters, usernames). Design documents describe only design goals, constraints, approaches, and implementation reasoning; they must read as this project's own design record.
- Why: the repository is publicly visible, and so are its history and PRs; provenance information and local paths must not leak with the repo.
- Where records go: provenance records worth keeping live in uncommitted local files (`outputs/` and `.worktrees/` are both in `.gitignore`).
- Companion requirement: example paths use neutral values (e.g. `C:\projects\my-app`), never real machine paths; existing content found to violate this rule is cleaned as an ordinary change, while traces already in history are handled separately.

## 2026-10-01 Compile-time work: merge test binaries, extract ProductStore, do not split runtime

- Status: active
- Decision: keep only 3 integration test binaries (`it`, `api`, `secret_authority_unarmed`), with new tests added to `tests/it.rs`; ProductStore and product contracts live in a separate `rove-product-store` crate; `apps/api/src/lib.rs` split into modules by responsibility; runtime is not split for now.
- Why: measured bottlenecks were the repeated linking of full-stack test binaries and the oversized api unit. Test builds after a runtime change dropped from 146s to about 66s. Data in docs/refactor-plan.md.
- Rejected: switching to the `rust-lld` linker, which made no measurable difference; disabling `rfd`/`rove-bench` via features, which only removed 6 crates with no compile-time change; splitting runtime now — submodules have cycles, and downstream crates depend on all submodules via Engine, so it would still trigger full rebuilds.

## 2026-10-01 Documentation restructured per the unified standard; old design docs deleted outright

- Status: active
- Decision: keep only AGENTS.md, CLAUDE.md, README.md, TODO.md, DESIGN.md, plus docs/architecture, api, decisions, development, user-guide, refactor-plan. The former docs/design, docs/plans, docs/Archive, docs/runtime, and root-level process logs were all deleted.
- Why: the old documentation had 125 files and about 70k lines, with the same thing written in multiple places and progress mixed into specifications. Also, older CLI versions only read CLAUDE.md and not AGENTS.md, so both files had to stay.
- Rejected: moving old docs to docs/archive — git history already preserves them, and keeping them in the repo only invites stale reads; keeping tests that assert doc content — they break on wording changes without proving any behavior.

## 2026-09-26 Attachments are restricted uploads; bytes live in the user data directory

- Status: active
- Decision: raw attachment bytes are stored at `<data_root>/attachments/<product_session_id>/<attachment_id>` with no extension, and metadata lives in ProductStore. Uploads have a size cap and a validated type.
- Why: user files must not be written into the workspace, and client-supplied filenames and MIME types are not trusted.
- Rejected: none recorded

## 2026-10-07 StreamEvent payloads join the OpenAPI surface

- Status: active
- Decision: `StreamEvent` and its payload graph derive `utoipa::ToSchema` in place (rove-models, rove-core, rove-runtime); `JobStreamEvent.event` references `StreamEvent` instead of a bare object, so the checked-in snapshot publishes all canonical event kinds. rove-protocol keeps its no-local-dependency rule: protocol IDs inside schema types are declared via `#[schema(value_type = String, format = "ulid")]` overrides rather than giving rove-protocol an utoipa dependency.
- Why: generated Web clients need the SSE payload contract in the spec, and the hand-written `STREAM_EVENT_NAMES`/type union already drifted once.
- Rejected: a schemars sidecar emitting only event schemas — it would create a second, drift-prone source of truth next to utoipa.

## 2026-09-17 Local HTML previews run on an isolated loopback origin

- Status: active
- Decision: executable HTML previews use a separate ephemeral port with no product cookies and no BearerAuth middleware.
- Why: scripts inside a preview page must not read the product token or call the product API as the user.
- Rejected: same-origin preview under the product origin, where page scripts could call the product API directly.

## 2026-08-24 Web UI V3: warm sand skin, layered only on tokens

- Status: active
- Decision: keep the v1/v2 DOM and class names; v3 only adds tokens and skins under `styles/v3/`, and structural changes go in `styles/product-v2.css`.
- Why: restyling does not require rewriting components or e2e selectors.
- Rejected: rewriting the component tree — high cost and a large regression surface.

## 2026-08-16 Hard read-only Review workflow

- Status: active
- Decision: Review captures an immutable Git target snapshot, runs on the same Engine with read-only tool and environment configuration, writes results to ProductStore, and does not occupy a chat turn.
- Why: the review process must not modify the workspace or contend with normal sessions for turns.
- Rejected: banning writes via the approval policy — not a hard boundary.

## 2026-08-16 Run data moved to the user-level data directory

- Status: active
- Decision: run state, memory, and the MCP catalog are isolated per workspace under `<data_root>/workspaces/<storage_key>/`; ProductStore is a single global store at `<data_root>/product.sqlite`. A legacy `.rove/` is only migrated explicitly via `rove state migrate`, dry-run by default, and conflicts are never overwritten.
- Why: run data must not pollute the project directory or get committed; different entry points must locate the same state.
- Rejected: auto-migrating on startup — risk of destroying the operator's data; following workspace moves and renames — high complexity with unclear boundaries.

## 2026-08-12 Provider config belongs to the user; projects may only select

- Status: active
- Decision: full provider profiles and credential references live only in `~/.rove/config.toml`. A trusted project may at most select an existing profile and model. Catalog writes use revision CAS. Plaintext keys are rejected. The runtime freezes a keyless model snapshot, and resume is refused when the provider drifts (`provider_changed_for_resume`).
- Why: project files must not become a source of credentials, and resume must not silently switch to a different model.
- Rejected: defining providers in project config — credential authority would scatter across projects; implicitly queueing model switches mid-run — hard to explain, so mid-run switches are refused outright.

## 2026-08-10 Desktop is a thin shell embedding rove-api

- Status: active
- Decision: the Tauri main process starts `rove-api` on a random loopback port, loads the exact same static bundle as Web, and exposes a minimal set of IPC commands. Desktop has no Engine, state store, or ProductStore of its own.
- Why: maintain one runtime and one UI only.
- Rejected: a Desktop-specific UI branch; a Desktop-owned backend.

## 2026-07-26 Restore the visible conversation after refresh

- Status: active
- Decision: after a Web refresh, restore the current session's full transcript from persisted canonical events. When only partial recovery is possible, show "partial" and the reason explicitly.
- Why: Web must be usable as the daily driver.
- Rejected: keeping resume capability without restoring chat bubbles — unacceptable for daily use; "soft stitching" the transcript — would break the continuity guarantee.

## 2026-07-26 ProductStore is a separate product control store

- Status: active
- Decision: the API has one global `product.sqlite` storing workspaces, sessions, preferences, bindings, and messages/control. It does not duplicate canonical events; transcripts are projected from each workspace's StateStore at read time.
- Why: the product catalog must span workspaces, while execution facts stay in their own workspace — there is no second source of truth.
- Rejected: putting product state into each workspace's `state.sqlite` — a cross-workspace catalog would then be impossible.

## 2026-07-24 Provider naming and the "delete all legacy" policy

- Status: active
- Decision: the product field is `provider_type` (`openai`, `openai-responses`, `anthropic`, `ollama`, `fake`); the system field is `wire_protocol`, mapped from the former and not user-settable. Since the product has not shipped, legacy implementations are deleted outright: the dual-track provider client, `plan_step_*` dual events, duplicate assembly entry points, `.rove/rag.lancedb`, etc.
- Why: one concept keeps one name and one path.
- Rejected: a long compatibility window; treating "OpenAI-compatible" as a product type — compatible gateways should use the `openai` type with a custom base URL.

## 2026-07-23 Provider layer: named profiles + explicit protocol

- Status: active
- Decision: multiple named profiles plus an `active` selection; the protocol is configured explicitly; credential sources start with env vars and files (keyring added later); model lists are only queried locally from each endpoint.
- Why: any compatible gateway and the official API go through the same mechanism — no guessing.
- Rejected: falling back to OpenAI for unknown protocols; inferring the protocol from URL substrings; using commands as a credential source — this one still awaits its own security review, never done; a remote model catalog service.

## 2026-07-22 Modular Cargo workspace; built-in vector RAG removed

- Status: active
- Decision: split into the one-way dependency chain `rove-models <- rove-core <- rove-runtime <- rove-app-bootstrap <- apps/*`, guarded by `tests/workspace_architecture.rs`. Removed the built-in vector RAG (LanceDB); workspace retrieval is now tools (`read_file`, `search_code`, `run_shell`) plus layered file memory.
- Why: the core can be embedded externally, and surfaces are just shells. RAG brought heavy dependencies and state with unclear benefit.
- Rejected: a single large crate; keeping RAG as a default feature. If semantic retrieval is ever needed, it must be designed as an optional external capability.

## 2026-07-15 MCP supports Streamable HTTP; rich results go through Tool Artifacts

- Status: active
- Decision: MCP supports stdio, legacy SSE, and Streamable HTTP. Results from all three transports map to the same restricted result envelope; binary content goes to a content-addressed Tool Artifact store and never enters prompts or events. An in-flight run pins the tool catalog from startup; `listChanged` only affects later runs.
- Why: context and event sizes are capped, and remote-supplied filenames and URIs must not become local paths.
- Rejected: hot-swapping the tool catalog mid-run — behavior would differ before and after within the same run.

## 2026-07-14 Execution lifecycle: rule-based Plan-Execute-Replan

- Status: active
- Decision: explicitly distinguish `react` and `plan_react` strategies. Each plan step is a bounded inner ReAct; a successful tool call does not equal a completed step. `StepRecord` is append-only; `PlanRevision` only replaces remaining steps; `PlanEvaluator` is rules-first with the model handling only typed ambiguity; the Finalizer draws conclusions independently from evidence; budgets are computed per dimension; existing trace/task-state/checkpoint storage is reused.
- Why: recoverable, auditable, and never reports failure as completion.
- Rejected: adopting LangGraph; a third `auto` strategy chosen by the model; building a second state system; exposing chain-of-thought in events.

## 2026-07-14 AgentDefinition and instructions layered by trust and path

- Status: active
- Decision: versioned AgentDefinition packages compile into immutable run profiles. Root `AGENTS.md` is stable policy; nested `AGENTS.md` applies only to matching paths, and the first tool call entering a new scope is refused (`precondition_required`), carrying that layer's instructions on the next turn. All workspace sources need their own Trust capability.
- Why: instruction text may enter context but can never grant permissions; recovery must not be affected by swapped source files.
- Rejected: none recorded

## 2026-06-09 OpenAPI generated from utoipa annotations

- Status: active
- Decision: API docs are generated at runtime from `#[utoipa::path]` and `ToSchema`, served via `/api/openapi.json` and `/swagger-ui`.
- Why: documentation lives next to route types and cannot drift.
- Rejected: a hand-written OpenAPI file.
