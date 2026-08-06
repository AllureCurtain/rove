# CDH Completion — Implementation Log

> Working log for porting + completing the cloud-produced "CDH completion" work
> (from the alder workspace copy) into rove proper. We work ONLY in this worktree.
> One In Progress item at a time. Failures are recorded, not deleted.
>
> Checklist plan: `docs/plans/2026-08-03-cdh-alder-merge.md`

## Session metadata

- **Start:** 2026-08-01 (local, China Standard Time)
- **Resumed:** 2026-08-03 — user confirmed the worktree is now `.worktrees/cdh-merge`; G8 Desktop out of scope; G7 = gap-fill on main Settings (not full alder port); alder root docs not copied.
- **Worktree:** `.worktrees/cdh-merge`, branch `worktree-cdh-merge`, from `main @ 3aa51a1`
- **OS:** Windows 11 (win32), shell = Git Bash
- **Source being reviewed/ported:** `D:\Study\cc\claw\code\CODING_TASK_36_ROVE\res\alder\workspace\rove` (no git history; treated as untrusted reference, reviewed file-by-file before porting)
- **Task guide:** `D:\Study\cc\claw\code\CODING_TASK_36_ROVE\CODING_TASK_GUIDE-36-ROVE.md`

## Rules for this port

- Cloud code is REFERENCE ONLY: read + review every file before porting; adapt to repo conventions (AGENTS.md); no blind copies.
- Schema migrations land in order 5→6→7→… and never renumber.
- Every group: server contract + persistence + UI + real tests. No server-only or UI-only debt.
- After each group: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, focused `cargo test`, and `pnpm test/typecheck/build` in `apps/web`.
- **G8 Desktop is Out of scope** for this program.
- **G7** improves gaps against main Settings; do not regress Web Complete C2.

## Task breakdown

| # | Task | Status |
|---|------|--------|
| 0 | Plan doc `docs/plans/2026-08-03-cdh-alder-merge.md` + log scope fix | Done |
| 1 | G1 Steer + Follow-up (port + server follow-up drain + queue UI) | Done |
| 2 | G2 Fork (port + terminal-boundary validation + inherited history + branch tree) | Done |
| 3 | G3 Session model/reasoning (port backend + picker UI) | Done |
| 4 | G4 usage/context/cost (port + pricing snapshots + context section) | Done |
| 5 | G5 files/artifacts/diff (port + artifact download/preview + image validation) | Done |
| 6 | G6 export (port + Markdown + canary redaction test) | Done |
| 7 | G7 settings gap-fill (main Settings baseline; alder MCP only if additive) | Done |
| 8 | G8 Desktop | **Skipped — out of scope** |
| 9 | Hardening: concurrency/crash/load + acceptance runner | Done |
| 10 | Final gates + product docs + cleanup + PR | Pending |

## Step 3 — G3 Session model + reasoning (2026-08-04, Done)

**Audit:** Product `/jobs` still accepted browser `model` / `max_steps` /
`provider` / `approval`. The only durable selection was global
`product_preferences`; Composer QuickModelControl wrote that global row. No
session model table, run snapshot, reasoning protocol option, or session model
API existed.

**Implemented:**

- Migration 008: `product_session_model_configs` (revision CAS) and append-only
  `product_session_run_models` snapshots captured at claim/bind time.
- Session config is seeded from preferences, forked with a fresh revision, and
  resolved server-side with stored provider profiles / key env vars.
- Product job assembly rejects client-supplied model/provider/max_steps/approval
  fields; OpenAI Responses alone receives validated `reasoning_effort`.
- Web: QuickModelControl reads/writes session config, turn request is
  server-owned, Settings global selection remains a seed only.
- Deleting a provider profile nulls live session config `profile_id` (revision
  bump) and historical run-model `profile_id` so cleanup cannot 500 on FK.

**Failures found and retained:**

- Live smoke cleanup failed with 500 while deleting a profile still referenced by
  `product_session_run_models.profile_id`. Fixed by nulling historical snapshot
  profile references while keeping model/max_steps/reasoning immutable.
- OpenAPI product routes for model-config / run-models / provider models omitted
  500/503 responses required by the contract test; documented to match other
  product routes.
- Clippy `-D warnings`: useless `format!`, manual `Default` impl, needless
  borrows, and `too_many_arguments` on session model writers. Fixed with
  `.to_string()`, derived Default, borrow cleanup, and a
  `SessionModelConfigWrite` struct.
- Steer/follow-up Playwright case still selected fake-raw only in Settings and
  never wrote the session model config, so `request_input` echoed as plain
  text. Updated the e2e to call `selectSessionModel` after profile create.

**Verification:**

- `cargo fmt --all --check` — passed.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed.
- `cargo test -p rove-api --lib -- --test-threads=1` — 85 passed.
- `cargo test -p rove-integration-tests --test api -- --test-threads=1` —
  87 passed (includes CAS + mid-run snapshot immutability + OpenAPI).
- `cargo test -p rove-models --lib` — 118 passed.
- `pnpm test` — 28 files / 187 tests. `pnpm typecheck` / `pnpm build` — passed.
- `scripts/integration-smoke.ps1 -ApiAddr 127.0.0.1:18891 -WebPort 13004
  -IntegrationRoot %TEMP%\\rove-cdh-g3-retry-20260804` — local-full 5/5.

**Status:** G3 is done. G4 usage/context/cost is next; G8 remains out of scope.

---

## Step 4 — G4 Usage / context / cost (2026-08-04, Done)

**Audit:** Inspector showed live SSE usage and hard-coded cost
`Unavailable`. No session usage API, no per-run pricing snapshot, and no
distinction between unpriced models and local/fake zero cost. Token totals
already existed in runtime `report.json`.

**Implemented:**

- Bundled pricing module with versioned source `bundled@2026-08-04.1`,
  availability classes `priced` / `local_zero` / `unpriced`.
- Migration 009 adds pricing columns on `product_session_run_models`; claim/bind
  freezes the snapshot so later rate edits cannot rewrite historical cost.
- `GET /product/sessions/{id}/usage` aggregates durable run reports, attaches
  frozen cost + latest context occupancy from prompt-build metadata, and keeps
  partial reasons when reports are missing.
- Web client + `useSessionUsage` + inspector session totals/cost/context wiring.
  Unpriced stays Unavailable; fake models show explicit local zero.

**Failures found and retained:**

- Initial OpenAPI/route wiring and clippy issues from G3 leftovers were fixed
  before G4 gates.
- Usage unit fixtures used an invalid ProductSessionId string; switched to
  `ProductSessionId::new()`.
- Fake provider reports zero tokens, so the integration test asserts local-zero
  cost classification and two-run rollup rather than non-zero token counts.

**Verification:**

- `cargo fmt --all --check` — passed.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed.
- `cargo test -p rove-api --lib` focused pricing/usage/store — passed.
- `cargo test -p rove-integration-tests --test api` — 88 passed including
  `product_session_usage_aggregates_report_totals_with_local_zero_cost`.
- `pnpm test` — 28 files / 187 tests. `pnpm typecheck` / `pnpm build` — passed.
- `scripts/integration-smoke.ps1 -ApiAddr 127.0.0.1:18892 -WebPort 13005
  -IntegrationRoot %TEMP%\\rove-cdh-g4-20260804` — local-full 5/5.

**Status:** G4 is done. G5 files/artifacts/diff is next; G8 remains out of scope.

---

## Step 5 — G5 Files / artifacts / Diff (2026-08-04, Done)

**Audit:** No product file/artifact/diff routes. Inspector only showed opaque
refs and forbade open/download labels.

**Implemented:**

- `GET /product/workspaces/{id}/files` and `/files/content` with join_safe path
  checks, secret-name rejection, symlink escape rejection, 1 MiB content cap.
- `GET /product/sessions/{id}/artifacts` lists durable report/task_state/trace
  and registered run `artifacts/` files with opaque ids.
- `GET /product/sessions/{id}/diff` aggregates tool mutations from run reports
  plus optional git name-status for repo workspaces.
- Web client parsers + FilesPanel/DiffPanel in the inspector.

**Correction (2026-08-06):** The earlier "binary/image partial" residual is
obsolete. Binary artifact download/stream and image format/byte/pixel validation
are implemented and covered by
`product_artifacts_are_hashed_session_bound_and_report_cleanup` and
`product_workspace_files_are_bounded_typed_and_safely_delivered` in
`tests/api.rs`. The G1-G6 re-audit found no remaining code gap here.

**Verification:**

- Unit tests for path safety / range / artifact names / diff scope.
- API tests: files traversal/secret rejection; artifacts after completed run.
- `cargo clippy --workspace --all-targets -- -D warnings` passed.
- `cargo test -p rove-integration-tests --test api` 90 passed.
- Web typecheck/test/build + local-full 5/5.

**Status:** G5 core Done. G6 export is next.

---

## Step 6 — G6 Evidence export (2026-08-05, Done)

**Audit:** No product session export route. Alder has JSON/HTML export with
redaction walker as untrusted reference.

**Implemented:**

- Added `POST /product/sessions/{session_id}/export?format=json|html|markdown`.
  All formats render from one sanitized evidence value containing lineage,
  inherited/local canonical transcript segments, controls, run-model snapshots,
  usage/cost/context, and an artifact manifest without artifact bytes.
- Redaction removes secret fields and token-shaped values, Authorization and
  environment values, workspace/home/temp paths, and hidden reasoning. String,
  aggregate-text, and final-response budgets are explicit and UTF-8 safe.
- Offline HTML is script-free and returned with CSP, `nosniff`, `no-store`, and
  attachment headers. The Web Inspector and Sessions Settings download the real
  server export in all three formats.

**Failures found and retained:**

- The first integration fixture tried to start a second run before the first
  supervisor released its session claim and received the correct 409. The final
  canary test exercises tools, input, Steer, Follow-up, and the automatic
  follow-up run through one ordered session lifecycle.
- A follow-up automatically transitions the session back to `idle`; the test
  originally waited for `needs_attention` and was corrected to assert the real
  durable state.

**Verification:**

- `cargo test -p rove-api --lib product:: -- --test-threads=1` — 81 passed.
- `cargo test -p rove-integration-tests --test api
  api_exposes_openapi_json_for_all_routes -- --exact --nocapture` — passed.
- `cargo test -p rove-integration-tests --test api
  product_session_evidence_export_is_complete_bounded_and_redacted_in_all_formats
  -- --exact --nocapture` — passed; JSON/HTML/Markdown contained none of the
  canary secret, environment value, Authorization token, or workspace path.
- `pnpm test -- product/evidence-export.test.ts inspector/RunInspector.test.ts`
  — 2 files / 10 tests passed; `pnpm typecheck` — passed.

**Status:** G6 is done. G7 Settings gap-fill is next.

---

## Step 7 — G7 Settings gap-fill / MCP (2026-08-06, Done)

**Audit:** Main Settings had no MCP surface. Product Jobs and Settings resolved
different MCP paths, so a catalog edited in Settings was not the catalog a
product Job loaded.

**Implemented:**

- Product Job assembly now forces the workspace-bounded MCP catalog
  (`assemble_job_engine` in `apps/api/src/lib.rs`), so Settings and Jobs agree.
- Workspace-scoped MCP CRUD plus probe under `apps/api/src/product/mcp.rs`, with
  raw `env` and secret-shaped arguments rejected and never echoed. Only
  `env_names` persist; values resolve at spawn time by name.
- Server names are restricted to `[a-z0-9_]+` so normalized tool prefixes cannot
  collide.
- Bounded reads across all MCP transports (`MAX_MCP_RESPONSE_BYTES` = 1 MiB) for
  stdio JSON lines, legacy SSE endpoint discovery, and SSE JSON responses. HTTP
  bodies accumulate in chunks instead of an unbounded `text()`/`json()` read.
  Empty tool names are rejected. These classify as protocol mismatch.
- `MCPSettings.tsx` with strict parsers, workspace isolation, typed probe errors,
  and drafts that survive a failed mutation; wired into `SettingsShell.tsx`
  Tools & Approvals.
- PowerShell + POSIX acceptance entries (`scripts/product-acceptance.ps1`,
  `scripts/product-acceptance.sh`) writing a real `PRODUCT_ACCEPTANCE_REPORT.json`.

**Failures found and retained:**

- `cargo fmt --all --check` and the focused MCP Rust tests were dispatched but
  never reported in the prior session, which ended on repeated HTTP 429. On
  resume the hardening tests passed 6/6, but `fmt` failed on two spots in
  `tests/mcp.rs` and `clippy` failed with `redundant_closure` in
  `runtime/src/tools/mcp_config.rs`. Both are fixed.
- The full `tests/api.rs` suite then failed one case:
  `api_registers_configured_mcp_tools_for_jobs` hung waiting for approval. The
  mock server declares `readOnlyHint: true`, and G7 deliberately stopped treating
  remote annotations as a local policy grant, so the tool is destructive locally.
  The test was stale against an intentional security change, not a code gap; it
  now grants approval explicitly and states why.
- The Settings Playwright suite exposed five stale G2/G3/G6 browser contracts
  (missing required `inherited: false` on transcript segments, the replaced
  global Preferences model control, client-sent `approval`/`max_steps` overrides
  that are now server-owned, and the renamed session export action). All were
  corrected against current server behavior rather than by relaxing assertions.
- The Tools page still labeled the global `max_steps` "Maximum steps per job"
  after G3 made per-run limits session-owned. Reworded to a new-session default.
- The first full acceptance sweep failed `clippy` with four style lints in
  `apps/api/src/product/export.rs` and `apps/api/src/product/files.rs`
  (`while_let_loop` x2, `manual_contains`, `explicit_counter_loop`). Fixed; the
  redaction canary and files/artifact contracts were re-run to prove the
  rewritten loops kept their bounds and semantics.
- The full browser sweep then exposed two more issues that `settings.spec.ts`
  alone could not see:
  - **Real regression (C1 fail-closed).** The G3 rewrite of
    `apps/web/state/turn-request.ts` correctly stopped sending client
    provider/approval/max_steps overrides, but it also deleted
    `ProviderSelectionError` and the profile-existence check. A dangling
    `provider_selection.profile_id` therefore submitted a turn and rendered an
    optimistic message. `validate_provider_selection` in
    `apps/api/src/product/store/validation.rs` only checks model text and
    `max_steps` range, so the server did not catch it either. The two concerns
    were separated: `assertProviderSelectionIsSatisfiable` now performs a local
    consistency check against the catalog the browser already holds and is called
    in `use-session-continuity.ts` before any optimistic append, while the request
    payload still carries no server-owned fields. Covered by four new cases in
    `apps/web/state/turn-request.test.ts`.
  - **Stale assertion.** `polish.spec.ts` asserted that Tab inside the mobile
    inspector returns focus to "Close run evidence". That only held while the
    close button was both the first and last focusable element. G5/G6 added the
    evidence-export controls, so Tab now advances within the panel, which is
    correct. The focus trap in `RunInspector.tsx` is generic and unchanged; the
    test now asserts the real invariant (focus stays inside, and Shift+Tab from
    the first control wraps to the last) instead of an element count.
- The PowerShell acceptance entry initially misreported `fmt` as failed because
  `Start-Process -PassThru` without `-Wait` left `ExitCode` unset, and it crashed
  on `[System.IO.Path]::GetRelativePath`, which does not exist on Windows
  PowerShell 5.1. Both fixed, plus BOM-free JSON output. An undeterminable exit
  code is now recorded as `error`, never as a pass. Schema parity between the two
  entries was verified by comparing generated reports.

**Verification:**

- `cargo test -p rove-integration-tests --test mcp` — 6 passed, including
  `mcp_sse_rejects_oversized_discovery_and_json_responses`.
- `cargo test -p rove-integration-tests --test api product_mcp_ -- --test-threads=1`
  — 4 passed, including
  `product_mcp_maps_corrupt_locked_and_unsafe_config_to_typed_conflicts`, which
  asserts corrupt JSON, a held lock, a non-regular-file path, and a symlinked
  catalog all map to `409 product_mcp_conflict`.
- New runtime contract tests
  `disabled_mcp_servers_are_never_assembled_or_environment_resolved` and
  `mcp_environment_resolution_rejects_invalid_and_unavailable_names` — passed.
- `pnpm typecheck` and focused MCP/Settings Vitest — passed.
- `pnpm test:e2e -- tests/e2e/settings.spec.ts` — 12 scenarios passed.
- Full gate results are recorded in `PRODUCT_ACCEPTANCE_REPORT.json`.

**Status:** G7 is done. G8 Desktop remains out of scope.

---

## Step 8 — Hardening + acceptance (2026-08-06, Done)

This row was briefly marked Done when only the acceptance runner existed, then
corrected to Partial, and is now genuinely complete. All three plan bullets have
executed evidence.

**Concurrency (`product_concurrent_multi_workspace_control_operations_stay_isolated_and_serialized`):**

Two workspaces, each holding a live run at a pending input, driven with
`tokio::join!` rather than sequentially:

- Steer and follow-up fired at both workspaces at once; controls stay strictly
  partitioned by session (each workspace's control list contains its own payloads
  and none of the other's).
- Concurrent duplicate submissions under one idempotency key return the original
  control id instead of creating a second.
- Two concurrent model-config writes with the same `expected_revision` produce
  exactly one `200` and one `409 product_session_model_config_conflict`; the
  assertion sorts the pair, so both-succeed and both-fail are equally rejected.
  Workspace B's config is proven untouched by A's contention.
- Both runs released simultaneously: each follow-up starts exactly one successor
  in its own session, and both sessions land on ordinal 2.
- Concurrent forks at each terminal boundary yield distinct children with correct
  parents; a repeated key returns the same child; each child appears only in its
  own workspace's session list.

**SSE drop (`api_sse_stream_dropped_mid_flight_loses_no_events_on_reconnect`):**

A live stream is opened against a run held at a pending input, read until at
least one identified event arrives, then the body is dropped mid-flight. That is
a client disconnect, not a clean close. The run then completes normally, proving
the severed stream did not disturb it. Reconnecting with `Last-Event-ID` returns
exactly the undelivered range: the replayed ids equal
`(dropped_at+1)..=event_count`, are strictly increasing, contain no already-seen
event, and include `run_completed`.

**Load smoke (`product_long_session_deep_tree_large_dir_and_large_diff_stay_bounded`):**

- Long session: six consecutive turns; ordinal advances exactly once per turn,
  every run id is distinct, and the transcript's segment ordinals are exactly
  `1..=6`.
- Deep tree: a 24-level nested prefix resolves to its leaf with
  `scan_limit_reached: false`, and traversal escape from that depth is still
  refused.
- Large dir: 250 entries paged at 100 per request; pagination terminates, covers
  every entry exactly once with no page overlap, and preserves sorted order.
- Large diff: 4,200 injected tool mutations against a 4,096 entry cap. The
  response is asserted to be *exactly* 4,096 entries, so an empty response cannot
  pass, carries a "capped" partial reason, and stays inside the 4 MiB total
  budget.

**Directory-scan bound, previously an untested edit:**

The `explicit_counter_loop` fix replaced a manual counter with `rd.enumerate()`,
and the `scan_limit_reached` branch had no coverage. Materializing
`MAX_DIRECTORY_SCAN` (50,000) files was measured at ~39s of pure fixture setup
while `read_dir` itself costs ~29ms, so the cost was all setup and unsuitable for
the default suite. `collect_entries` now takes `scan_limit` as a parameter (the
route passes `MAX_DIRECTORY_SCAN`), making the bound testable in milliseconds:

- `directory_scan_stops_at_the_limit_and_reports_it` — under the limit returns
  everything unflagged; at the limit returns exactly `scan_limit` entries and
  reports the cut; a zero limit collects nothing rather than scanning.
- `directory_scan_limit_does_not_count_skipped_secret_entries_as_results` —
  secrets are consumed by the scan but never returned, so a spanning limit yields
  fewer results without misreporting the cut.

**Failures found and retained:**

- `CreateJobResponse` is not `Clone`; the concurrency test consumes the vector by
  iterator instead.
- Sixteen `E0716` errors: a `format!` temporary cannot outlive a `tokio::join!`
  arm. All request URIs are now bound before the join.
- The fork response nests the child at `["session"]["id"]`, not
  `child_session_id`. Corrected, and parent linkage is now asserted too.
- The two new `collect_entries` tests first returned zero entries: the route
  canonicalizes the workspace root before scanning, and on Windows
  `canonicalize` yields an extended (`\\?\`) path, so containment failed against
  a raw temp path. The tests canonicalize the root like the route does.

**Weaknesses found by auditing the new tests themselves:**

These passed on the first run but did not prove what they claimed. Both were
strengthened, and the probes below were removed after confirming the values.

- The concurrency test ran on the default single-thread `#[tokio::test]` runtime,
  where `tokio::join!` only interleaves at await points. It now uses
  `flavor = "multi_thread", worker_threads = 4`. Probing the CAS pair across
  repeated runs showed the winner genuinely alternates (`first=200/second=409`,
  then `first=409/second=200` twice), which is why the assertion sorts the pair
  instead of pinning an order. Five consecutive multi-thread runs passed.
- The same test discarded the `steer_b` response. A server that returned one
  shared control id for both workspaces would therefore still have passed the
  idempotency assertions. It now asserts all four concurrently created control
  ids are distinct, and that a replay is never confused with the other
  workspace's control.
- The SSE drop test never proved the run was still live at the drop. It could
  have been severing an already-finished stream, which would only exercise
  replay-after-close. It now asserts `Running` status, a still-pending input, and
  the absence of `run_completed` in the bytes read before dropping. A probe
  confirmed the drop happens at event 1 of 17, so the reconnect closes a real
  16-event gap rather than a trivial one.

**Verification:**

- `cargo test -p rove-api --lib product::files` — 8 passed.
- The three new integration tests pass individually and in the full suite.
- `cargo fmt --all --check` and
  `cargo clippy --workspace --all-targets -- -D warnings` — passed.
- Full gate results are in `PRODUCT_ACCEPTANCE_REPORT.json`.

**Inherited scope note:**

- G1-G6 "no remaining code gap" is carried forward from the prior session's
  re-audit. This session verified G7, the hardening items above, and the full gate
  sweep; passing gates prove existing tests hold, not that they cover every
  G1-G6 requirement.

---

## Step 0 — Plan + scope (2026-08-03)

**What:** Wrote `docs/plans/2026-08-03-cdh-alder-merge.md`; marked G8 skipped; G7 as gap-fill; confirmed worktree tip = main `3aa51a1` with partial uncommitted G1 runtime/contracts already present.

**Why:** User confirmed merge worktree and ordered full planned groups except Desktop; need a durable checklist separate from alder’s oversold delivery docs.

---

## Step 1 — G1 Steer + Follow-up

### 1.1 Review notes (cloud code read before porting)

- `runtime/src/engine/control.rs` — clean: `SteerId`, `SteerMessage`, `RunControlHandle` (cloneable, Option<Sender>), `control_channel()` bounded 64, `drain_steer`. Unit tests included. Steer-only by design; follow-up is an API/ProductStore concern.
- `facade.rs` — `RunStream` gains `control()` accessor; `control_channel()` created per run in `Engine::run_with_cancel`; receiver threaded into `LoopContext` as `Option<SteerReceiver>` (Arc<AsyncMutex<Receiver>>).
- `run_loop.rs` — safe-point drain at top of step iteration before prompt construction; pushes `Message::user(content)` into working memory and yields `SteerAccepted`; on cancel drains remaining and yields `SteerDropped`. `plan_loop.rs` needs the same (cloud did both — check).
- `events.rs` — 5 new variants + names. Purely additive; matches cloud.
- `product_session_controls` (MIGRATION_005) — id/kind/idempotency_key/digest/content/status/run_id/seq/timestamps; unique partial index (session, idempotency_key); indexes (session,status,created_at) and (session,seq).
- Repository: `create_control` idempotent (same key+same digest → replay same row; same key+different digest → ProductControlConflict), `list_controls`, `get_control`, `transition_control` (CAS from→to), `abandon_pending_controls`, `list_pending_followups`.
- routes.rs — steer: persist first, then non-blocking `try_send` to live job control handle. followup: persist; delivery is durable-only (supervisor drain). revoke: CAS pending→revoked.
- lib.rs — JobRecord gains `product_session_id`, `product_store`, `control: Mutex<Option<RunControlHandle>>`; consume_job_stream captures/releases handle and reflects Steer/Followup lifecycle events back to store; supervisor abandons pending controls on non-final termination.
- Web — types + client methods; Composer busy-mode Steer/Follow-up buttons; continuity steer/followup callbacks; ProductApp wiring; rove-state handles 5 new events.

### 1.2 Gaps found in cloud (to fix after port)

- `list_pending_followups` has ZERO call sites — queued follow-ups never auto-start the next run server-side. Web fakes it with `if idle → send(content)` which double-submits if the server ever drains. MUST implement server drain: on terminal Done → claim next pending follow-up in seq order → start new run with same product session → mark FollowupDequeued; crash/restart safe (no loss, no double-start); on cancel/error/needs-attention → leave pending abandoned for user confirmation.
- No follow-up queue UI panel (only enqueue buttons). Add pending-controls list with real revoke.
- FollowupQueued/Dequeued/Abandoned events almost never emitted as producers.

### 1.3 Porting log

#### Already present (uncommitted from prior session, 2026-08-01)

- Runtime: `control.rs`, events, run_loop/plan_loop/facade/mod wiring.
- API contracts: ProductControl* types + ProductControlConflict/Rejected error codes.
- Schema still v4 — MIGRATION_005 not applied yet.

#### Next actions (this session)

1. Finish ProductStore trait methods + MIGRATION_005 + repository + store tests.
2. Routes for steers/followups/controls/revoke; wire lib JobRecord + event reflection.
3. Supervisor: abandon on non-final; on Final claim+start follow-up; restart drain.
4. Web types/client/Composer/continuity/queue UI; remove idle double-send.
5. fmt/clippy/test gates for G1.

_(continues below as steps complete)_

### 1.4 Resume audit (2026-08-03)

**What:** Confirmed Git's registered worktree location is now `.worktrees/cdh-merge`; no source files were moved or reset in this audit. The worktree contains uncommitted G1 runtime/API/store changes plus this plan and log. The repository root has a separate user-owned `README.md` modification that is intentionally untouched.

**Observed verification:**

- `cargo check -p rove-api` — exit 0.
- `cargo test -p rove-api product::store::tests --lib -- --test-threads=1` — exit 0; 28 passed.

**Current G1 gap:** backend control persistence and scheduler code exist, but the production Web client/composer/transcript projection has not yet been wired to the controls API. G1 remains **In Progress** until the real Web flow and integration/crash-boundary coverage are complete.

### 1.5 Continuation audit (2026-08-03)

**What:** Confirmed that the registered `worktree-cdh-merge` checkout is
`.worktrees/cdh-merge` at `main @ 3aa51a1`; `.claude/worktrees` contains no
active CDH checkout. Reviewed the current G1 runtime, ProductStore, API, and
production Web changes before making further edits.

**Observed state:** The Store control-state tests and the runtime control unit
tests pass, and `cargo check -p rove-api` passes. The API has durable control
routes and follow-up drain code, but OpenAPI component/route coverage, real
API lifecycle coverage, and the production Composer/continuity wiring remain
unfinished. G1 is still **In Progress**.

**Next:** Close terminal/startup exceptional paths without leaving controls
unclassified, then wire server-backed controls into the production Web shell
and add focused API/Web regression coverage before starting G2.

### 1.6 G1 completion pass started (2026-08-03)

**What:** Re-read the task guide, current runtime/API/Web contracts, and the
uncommitted G1 implementation in the registered `.worktrees/cdh-merge`
checkout. The worktree registration is correct; no migration from
`.claude/worktrees` is needed. Claude Code's recurring task is explicitly out
of scope for this implementation pass.

**Observed baseline:** `cargo check -p rove-api` exited `0` from this
worktree. The remaining G1 work is not cosmetic: the control API lacks full
OpenAPI declarations and API contract coverage, the cancellation completion
wait has a supervisor-registration race, and the production continuity hook
has only control state scaffolding without restore, terminal reconciliation,
or composer callbacks.

**Next:** Fix the lifecycle/contract gaps first, then finish the actual
Product UI wiring and run focused Rust/Web verification. Full outputs will be
captured under `verification-logs/` once final group gates are run.

### 1.7 G1 complete (2026-08-03)

**Implemented:**

- Added the bounded runtime steer channel and safe-point delivery in unplanned,
  planned, and multi-turn planned-step loops. Steers produce distinct accepted,
  applied, and dropped canonical facts; dropped steers are not added to resumable
  history as if they shaped a model turn.
- Added ProductStore schema migrations 005/006 for idempotent per-session
  controls and recoverable follow-up delivery. API routes persist/list/revoke/
  confirm controls, reflect lifecycle events back to the store, and publish the
  same canonical events through trace/SSE.
- Implemented atomic final-turn follow-up claim/start, startup drain of safely
  pending idle sessions, and conservative non-final/recovery classification.
  Browser code uses the server queue, never fakes an idle follow-up send.
- Wired the production Composer and continuity hook for Steer/Follow-up modes,
  durable queue/revoke/confirmation, and canonical event projection. The Stop
  action remains available while either control mode is active.

**Failures found and retained:** The first live browser run exposed that the
new control-mode default hid `Stop run`, making an approval-waiting run
uncancellable. The Composer now renders the Stop action beside the control
submission action. That exposed an existing broad `Run cancelled` test locator
which matched both the run status and a tool detail; the assertion now targets
the status value structurally. Both failures were reproduced and then passed in
the final live run.

**Verification:**

- `cargo fmt --all --check` — passed.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed.
- `cargo test --workspace` — passed.
- `pnpm test` — 28 files / 188 tests passed.
- `pnpm typecheck` and `pnpm build` — passed.
- `scripts/integration-smoke.ps1 -ApiAddr 127.0.0.1:18888 -WebPort 13001
  -IntegrationRoot C:\Users\AllureLove\AppData\Local\Temp\rove-cdh-g1-retry4-20260803`
  — local-full passed 4/4 Playwright cases, including live steer and queued
  follow-up revoke.

**Status:** G1 is done. G2 is the next in-progress group; G8 remains out of
scope.

---

## Step 2 — G2 Session Fork + branch tree (2026-08-03, Done)

### 2.1 Audit and implementation direction

**Reference review:** Alder's `product_session_forks` implementation records a
parent/child row and accepts an optional run/sequence, but it does not prove
that the source run is final and durable, does not create a new runtime
session/job identity for the child, and only comments about inherited
transcripts without implementing them. Its foreign-key cascade also erases
fork provenance when a parent session is deleted. It is therefore reference
material only, not a safe port.

**Chosen contract:** A fork will be created only from an API-verified final
runtime boundary. The ProductStore will retain immutable references to the
ancestry's runtime runs and exact terminal boundary; it will never copy parent
canonical events into a child ProductStore run ledger. The child's first turn
will seed a fresh runtime session/job from the validated source task state,
then own only its own subsequent bindings, controls, cancellation, and
follow-ups. Fork provenance must outlive parent-session deletion, while
workspace and runtime identity checks remain fail-closed.

**Implemented:**

- Added ProductStore migration 007 and immutable fork provenance records. A
  child stores its parent product session, source runtime session/job/run, and
  exact terminal event sequence; replay remains recoverable after the parent
  session is deleted.
- Added idempotent `POST`/`GET /product/sessions/{session_id}/forks` routes,
  OpenAPI contracts, and server-side terminal/run/task-state validation. Active,
  incomplete, and corrupt sources are rejected with typed product errors.
- A fork's first turn uses the validated source TaskState only as a prompt and
  history seed. It receives a fresh runtime SessionId, JobId, and RunId and is
  deliberately not recorded as `resumed_from_run_id`; later child turns resume
  only the child lineage.
- Canonical transcript projection now renders inherited source runs as read-only
  segments without copying their events into the child ledger. Local transcript
  ordinal validation remains based on persisted child bindings and presentation
  offsets are applied only after validation.
- Added the production Fork action, inherited-history labeling, parent/child
  Workspace tree, persisted fork-point display, and typed API/client/reducer
  handling. The tree is bounded by the ProductStore's session collection limit.

**Failures found and retained:**

- The first child turn originally returned `product_session_resume_conflict`:
  `new_job_record` inferred a normal resume relation from the reused source
  TaskState. The record construction now accepts the lineage explicitly, so a
  fork bootstrap preserves history but has no parent resume relation.
- The first successful child turn produced a partial transcript. The reader had
  shifted child binding ordinals for the inherited display prefix before
  validating local one-based ordinals. It now validates raw bindings before
  applying the display offset.
- One focused Vitest command initially used repository-relative paths while
  running from `apps/web`, so Vitest found no files and exited 1. The corrected
  paths ran 33 focused tests successfully; the full suite also passed.

**Verification:**

- `cargo check -p rove-api` — passed.
- `cargo test -p rove-integration-tests --test api product_session_fork -- --nocapture`
  — passed; 2 tests.
- `cargo test -p rove-api product::store::tests --lib -- --test-threads=1`
  — passed; 31 tests.
- `cargo test -p rove-api product::transcript --lib -- --test-threads=1`
  — passed; 6 tests.
- `cargo fmt --all --check` and
  `cargo clippy --workspace --all-targets -- -D warnings` — passed.
- `pnpm test` — passed; 28 files / 190 tests. `pnpm typecheck` and `pnpm build`
  — passed.
- `scripts/integration-smoke.ps1 -ApiAddr 127.0.0.1:18889 -WebPort 13002
  -IntegrationRoot %TEMP%\\rove-cdh-g2-20260803` — local-full passed 5/5
  Playwright cases, including a completed parent, UI Fork, child continuation,
  refresh, branch tree, and inherited transcript check.

**Status:** G2 is done. G3 Session model/reasoning is next; G8 remains out of
scope.
