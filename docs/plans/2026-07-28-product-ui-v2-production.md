# Product UI V2 Productionization Plan

> Status: **Current Scope Implemented And Verified In Worktree / Not Merged**
>
> Date: 2026-07-28
>
> Planning baseline: `30a345b4e0ef6c0534922192a4d90f86a0da81d0`
>
> Current worktree: `.worktrees/product-ui-v2-production`
>
> Current branch: `feature/product-ui-v2-production`
>
> Evidence record: [Section 14](#14-current-scope-evidence-record)
>
> Target design:
> [`../design/2026-07-27-product-ui-v2-finesse-design.md`](../design/2026-07-27-product-ui-v2-finesse-design.md)
>
> Current runtime truth:
> [`../runtime/README.md`](../runtime/README.md),
> [`../runtime/subsystems.md`](../runtime/subsystems.md),
> [`../runtime/implementation-guide.md`](../runtime/implementation-guide.md), and
> [`../runtime/implementation-status.md`](../runtime/implementation-status.md)

This plan migrates the approved Ice Steel Instrument Product UI V2 into the
real production `ProductApp` and fully connects every capability already
available through the current real API. It does not continue the inert
`/dev/product-ui-v2` Mock.

This worktree has a strict boundary:

- it owns the production Web interface, presentation state, Web read models,
  current API clients, and browser tests;
- it does not extend `runtime/**`, `apps/api/**`, ProductStore, generated
  OpenAPI, provider/runtime assembly, or Rust public contracts;
- a capability absent from the current API is documented here with its required
  contract, interaction states, and final acceptance scenario, then assigned to
  the subsequent **Control Capabilities and Desktop Dual-Host Implementation**
  program;
- an absent capability is never represented by Mock state, a dead button, an
  optimistic success, browser-local authority, or a private event lifecycle.

The later program is an explicit owner, not an unassigned backlog. Full Pi Web
capability-matrix production acceptance occurs only after this Web
implementation has merged, the later contracts and dual-host adapters are
implemented, and the complete real end-to-end gate passes.

The branch contains an in-progress UI-V2 implementation, but writing this plan
or landing branch-local code does not mark a capability row as accepted. Only
the evidence gates in Sections 10 and 13 can do that.

---

## 1. Fixed Decisions And Invariants

### 1.1 Visual decision is closed

The current worktree implements the confirmed visual design; it does not
redesign it. The design read remains: calm, forensic, reliable, and decisive;
`SOUL=7`, `SPECTACLE=2`, `DENSITY=8`, no hero engine, and feedback-only
motion.

The production implementation uses the approved Ice Steel light/dark palettes,
steel-cyan execution signal, compact fixed typography, `3 / 5 / 8px` geometry,
and evidence-spine information hierarchy. It does not introduce gradients,
glows, glass, decorative blobs, hero composition, giant type, viewport-scaled
type, or a new visual direction.

The operational interface must retain:

- stable desktop and mobile tracks;
- independent semantic success, warning, and failure colors;
- fixed loading geometry and no content-driven layout shift;
- visible keyboard focus and correct focus return;
- 44px minimum touch targets on mobile;
- complete reduced-motion terminal states;
- no document overflow, clipped labels, or overlapping controls.

These rules are stated directly so implementation does not depend on an
external agent skill to understand the acceptance criteria.

### 1.2 Runtime and product authority is fixed

The following are release blockers:

- Workspace, Session, Preferences, provider profiles, provider selection, and
  active product state remain API authoritative.
- Every turn uses the exact `product_session_id`; the server resolves its exact
  runtime binding. The browser never falls back to workspace-global `latest`.
- Canonical runtime events remain the only execution lifecycle shared by
  persistence, SSE, transcript, Inspector, reports, and tests.
- Browser read models may group or format canonical facts, but may not create a
  second writable event stream.
- `trace.jsonl` remains event fact, `task_state.json` remains resumable state,
  and `report.json` remains a derived summary.
- C4 workspace-scoped Memory, failed-cancel observation, run/sequence ordering,
  replay deduplication, and interleaved transcript behavior are protected.
- Approval and tool execution remain behind the shared ToolRegistry/safety
  boundary.
- Completed work and mutations are not replayed on resume. Unknown in-flight
  effects remain explicitly uncertain.
- Raw provider keys never enter browser state, requests, events, exports,
  screenshots, fixtures, or normal logs.
- Local deterministic Fake-provider execution remains available without
  network access or provider credentials.

### 1.3 Mock separation is fixed

Production modules must not import anything under:

```text
apps/web/app/dev/product-ui-v2/
```

The preview remains inert, `noindex`, and isolated. Its fixtures, session IDs,
tool results, approval actions, Steer/Follow-up affordances, provider states,
Memory states, export states, and Desktop states are not production inputs.

---

## 2. Audit Record

### 2.1 Production path

The audited production path on the planning baseline is:

```text
apps/web/app/(product)/layout.tsx
  -> ProductApp
     -> M1MigrationGate
        -> ServerProductApp
           -> useServerProductState
           -> useProductRouteSync
           -> useSessionContinuity
              -> createRunController
              -> workbenchReducer
              -> selectTranscriptTimeline
```

Relevant source includes:

- [`../../apps/web/shell/ProductApp.tsx`](../../apps/web/shell/ProductApp.tsx)
- [`../../apps/web/state/use-server-product-state.ts`](../../apps/web/state/use-server-product-state.ts)
- [`../../apps/web/state/use-product-route-sync.ts`](../../apps/web/state/use-product-route-sync.ts)
- [`../../apps/web/state/use-session-continuity.ts`](../../apps/web/state/use-session-continuity.ts)
- [`../../apps/web/api/run-controller.ts`](../../apps/web/api/run-controller.ts)
- [`../../apps/web/lib/rove-state.ts`](../../apps/web/lib/rove-state.ts)
- [`../../apps/web/state/transcript-projection.ts`](../../apps/web/state/transcript-projection.ts)
- [`../../apps/web/product/product-api-types.ts`](../../apps/web/product/product-api-types.ts)
- [`../../apps/web/chat/Transcript.tsx`](../../apps/web/chat/Transcript.tsx)
- [`../../apps/web/chat/Composer.tsx`](../../apps/web/chat/Composer.tsx)
- [`../../apps/web/inspector/RunInspector.tsx`](../../apps/web/inspector/RunInspector.tsx)
- [`../../apps/web/settings/SettingsShell.tsx`](../../apps/web/settings/SettingsShell.tsx)

The migration preserves this authority chain. Presentation is replaced from the
leaves inward; `useServerProductState`, route sync, continuity, controller, and
canonical reducer are not reimplemented under `product-v2`.

### 2.2 Reconciled design contradiction

The target design predates the integrated C4 foundation. Its as-built gap table
says transcript chronology is lost, and its implementation sequence says to
repair Memory, cancellation observation, and ordered projection first. That is
stale on this baseline.

Commit `0046e34` is present through the baseline and current code/tests prove:

- `selectTranscriptTimeline` sorts by run ordinal and event sequence;
- replayed sequences do not duplicate messages, tools, approvals, inputs, or
  plan facts;
- restored segments replay independently even when sequence numbers repeat
  across runs;
- Memory routes are scoped by server-owned `ProductWorkspaceId` and fail closed
  across workspaces;
- a failed cancel request leaves the focused `EventSource` attached.

Current code and runtime documentation therefore win. This worktree protects
those behaviors and improves their presentation; it does not invent a new
transcript API or browser lifecycle.

### 2.3 Current ProductApp limitations

The current Web reducer and components expose less than the current API already
provides:

- rich tool args, output, mutations, Diff, and execution metadata are collapsed
  into a short `details` string;
- `LlmMessage.usage` and `PromptBuilt.metadata` are formatted into trace text
  rather than retained as structured presentation facts;
- assistant content is rendered as plain text;
- tool invocation, approval, result/failure, scope, and mutation evidence do
  not read as one forensic unit;
- the current composer is disabled for `running` and `needs_attention`, which
  is correct until durable control APIs exist but is not clearly explained;
- provider/model selection is real and API-persisted, but only exposed deeply
  in Settings;
- the existing export is deliberately catalog metadata only.

These are the principal current-worktree targets.

---

## 3. Current API Capability Baseline

### 3.1 Fully usable by this worktree

| Current contract | Production Web responsibility |
|---|---|
| ProductStore Workspace/Session catalog and active selection | Render, mutate, retry, and reconcile through existing product clients only |
| Exact product-session/runtime binding and single active turn | Preserve exact IDs in every send, restore, attach, cancel, and display path |
| Canonical transcript with complete/partial/error state | Render one ordered production timeline and never substitute an empty success |
| Focused SSE attach/reattach and background status polling | Keep one focused `EventSource`, reject stale updates, retain background badges |
| Ambiguous job-start reconciliation | Show uncertainty and bounded reconciliation; never auto-submit a duplicate turn |
| Approval, input, cancel, and hard resume | Re-present all states and actions without changing semantics |
| Failed-cancel continued observation | Keep the Stop failure visible while the current stream remains observed |
| M1 migration and deep-route remapping | Keep `M1MigrationGate` before catalog reads and preserve exact retries/routes |
| Preferences revision CAS and provider selection | Provide real Settings and a quick provider/model control with honest global scope |
| Provider profile CRUD/test/model listing | Use existing clients, key-env references, loading/conflict/error states |
| Workspace-scoped Memory and runtime health | Complete V2 Settings presentation over existing bounded APIs |
| Session rename/delete and safe catalog export | Keep real management actions and label the export accurately |
| Ordered event projection and replay deduplication | Preserve run ordinal/event sequence authority in the V2 timeline |
| Tool call/result/mutation/Diff/execution metadata in current wire facts | Retain and render the full payload without a backend change |
| `LlmMessage.usage` and `PromptBuilt.metadata` | Retain structured token/approximate context facts for live and restored runs |
| Report total usage and tool mutations where already fetched | Cross-check the visible run evidence without treating report as event truth |

### 3.2 Not provided by the current API

The current API does not provide:

- durable Steer or Follow-up receipts/application semantics;
- product session parentage, tree, or runtime fork;
- session-scoped model defaults or a normalized reasoning-effort option;
- provider/model context-window capability or trustworthy price metadata;
- a product workspace file browse/read/preview endpoint;
- a general Tool Artifact manifest/download endpoint;
- a safe local image endpoint;
- a complete server-prepared evidence export;
- a Desktop host or host capability adapter.

This worktree must not create browser substitutes for those gaps. Their unified
follow-up assignment is specified in Sections 5 and 8.

---

## 4. Pi Web Capability Matrix

The matrix below is the scope and acceptance authority. Ownership labels are:

- **UI-V2:** this `product-ui-v2-production` worktree, using current real APIs.
- **CDH:** the later Control Capabilities and Desktop Dual-Host Implementation.
- **FINAL:** the real end-to-end production seal after UI-V2 and CDH merge.

| Capability | Current API truth | UI-V2 delivery now | Required missing contract | Owner | Acceptance authority |
|---|---|---|---|---|---|
| Ordered transcript | Run ordinal/event sequence ordering and dedup are implemented | Render one Trace Rail with run boundaries, partial notices, approvals and inputs in canonical order | None | UI-V2 | Live current API restore/SSE/replay assertions |
| Tool invocation/result grouping | Start, approval, completion/failure, result, mutation, Diff and metadata facts exist | Retain full fields and group by run-qualified `call_id` at the earliest canonical position | Artifact refs require later contract | UI-V2, CDH for artifacts | Live built-in tool and approval/input scenario |
| Markdown | Assistant text exists | Safe GFM renderer; raw HTML disabled; loading/error/plain-text fallback | None | UI-V2 | Real restored assistant response plus XSS negatives |
| Code | Assistant text exists | Inline/fenced code, language label, copy command, lazy highlighting, bounded blocks | None | UI-V2 | Real response, long code, keyboard and mobile tests |
| Diff | `ToolMutation.diff` exists | Structured unified-Diff summary/view from canonical mutation facts | Oversized Diff artifact/download contract | UI-V2, CDH for large Diff | Live write tool Diff agrees with canonical event/report |
| Images | No safe product image bytes endpoint | Render remote/local image references as explicit blocked/unavailable content, not `<img>` success | Same-origin workspace/artifact raster endpoint with MIME/size/pixel/range limits | CDH | FINAL real PNG/JPEG workspace/artifact preview and negative tests |
| Mermaid | Text content exists; no renderer | Strict lazy sandboxed rendering with accessible source fallback and no external fetch | None | UI-V2 | Sanitization/CSP/reduced-motion tests against real transcript text |
| Steer | Only an in-memory core queue; no product API | Do not render Steer action. Active composer shows accurate running/attention state and Stop only | Durable receipt, ordering, safe-boundary apply, canonical accepted/applied/blocked facts, restart/cancel semantics | CDH | FINAL real active-run apply-once/restart test |
| Follow-up | Only an in-memory core queue; no product API | Do not render Follow-up action or imply queued success | Durable ordered queue, cancel/confirm, exactly-once successor product turn | CDH | FINAL two queued turns, restart, cancel/error confirmation |
| Session tree | Product catalog is flat | Render the real flat workspace/session catalog; no fake indentation/parentage | Additive acyclic lineage read contract | CDH | FINAL restart-persistent parent/child tree |
| Fork | No product/runtime fork API | Do not render a Fork button | Idempotent terminal-boundary fork, new runtime identity, provenance, no mutation replay | CDH | FINAL parent/child divergence and replay-negative test |
| Quick model | API-persisted global `provider_selection` exists | Add compact provider/model control near composer, explicitly scoped as the next-run global default, using current Preferences CAS | Session-scoped run-default contract for per-session behavior | UI-V2 now; CDH for session scope | Current real Preferences/next-run test; FINAL session-isolation test |
| Reasoning control | No normalized option or provider capability | Show a non-actionable `provider default` fact only where useful; no button/menu | Normalized supported-effort capability and provider mapping; hidden reasoning remains excluded | CDH | FINAL supported request-shape and unsupported-reason tests |
| Context visibility | `PromptBuilt.metadata.token_estimate` and compaction facts exist | Show approximate token estimate, included/dropped history, compaction/degraded state | Context-window capability needed for an honest percentage/remaining amount | UI-V2 now; CDH for limit | Current live/restore fact equality; FINAL known/unknown limit test |
| Token/usage | `LlmMessage.usage`, step usage, and report total exist | Show prompt/completion/cached/total by canonical run without replay double count | Optional bounded historical aggregate only if current transcript/report proves insufficient | UI-V2; CDH only if a new aggregate is required | Current live/restore/report consistency test |
| Cost | Cost enforcement/price metadata is absent; zero fields are not price evidence | Show `Cost unavailable` as an honest state, never `$0` or an estimate | Server-owned versioned price metadata and immutable per-run cost snapshot | CDH | FINAL configured estimated, local zero, unknown unavailable, history immutability |
| Workspace files | No product browse/read endpoint | Show mutation paths as evidence text; do not make them fake links | Workspace-ID-based paginated list and bounded text/raw read | CDH | FINAL traversal/symlink/cross-workspace negatives plus real view |
| Tool Artifacts | No general artifact endpoint | Show only canonical refs as text if present; no preview/download action | Opaque artifact ID, manifest, MIME/size/hash, authenticated preview/download | CDH | FINAL real artifact survives restart and matches tool/report |
| Complete export | Current browser export is catalog metadata only | Preserve and restyle it under the exact label `Catalog metadata export` | Idempotent server export job with transcript, lineage, canonical refs, metrics, Diffs, artifacts and redaction report | CDH | FINAL archive cross-check, restart, redaction and size tests |
| Desktop dual host | No `apps/desktop` | No Desktop controls or simulated host status in production Web | Tauri host and host-capability adapter over the same product transport/state | CDH | FINAL separate Web/Desktop host smoke over identical IDs/contracts |

No Pi capability is unowned. UI-V2 closes all current-API presentation and
integration work; CDH owns every missing contract and dual-host adapter; FINAL
owns the combined real acceptance.

---

## 5. Follow-Up Contract And Interaction Handoff

This section is a binding handoff to CDH. It describes the minimum contract and
state model needed to finish the matrix. It is not authorization for this
worktree to edit Rust/API/ProductStore/OpenAPI.

### 5.1 Steer and Follow-up

Required server authority:

```text
instruction receipt
  receipt_id
  product_session_id
  mode: steer | follow_up
  server sequence
  request digest / idempotency identity
  expected job and run identity
  predecessor run ordinal
  state: accepted | applied | blocked | awaiting_confirmation | canceled
  accepted/applied timestamps
  canonical event reference or safe reason code
```

Required behavior:

- persist receipt before acknowledging submission;
- reject stale session/job/run preconditions;
- same idempotency key/body returns the same receipt, while a different body is
  a typed conflict;
- apply Steer only at a runtime-declared safe boundary before the next prompt,
  never during an unknown tool side effect;
- block unapplied Steer on interruption and never carry it into resume;
- order Follow-ups server-side and create exactly one successor product turn
  through the existing claim/bind path;
- after cancel, error, interruption, or uncertain effect, move Follow-up to
  `awaiting_confirmation` rather than executing silently;
- persist canonical accepted/applied/blocked facts; the browser does not emit
  them.

Required UI states after CDH lands:

- idle, submitting, accepted, applied, queued, canceling, canceled;
- stale conflict, ambiguous acceptance, blocked, awaiting confirmation;
- disabled with an exact server reason when the selected run is ineligible.

Required final scenario:

1. submit Steer during a real active run and prove one safe-boundary application;
2. reload and restart without losing or replaying the receipt;
3. queue two Follow-ups and prove exact server order and successor bindings;
4. cancel/error the predecessor and prove confirmation is required;
5. verify all receipt, product session, job, run, ordinal, and event identities.

### 5.2 Session lineage and fork

Required server authority:

- immutable `parent_product_session_id` and exact terminal fork boundary;
- bounded depth and acyclic validation;
- idempotent fork receipt;
- new product and runtime session identities;
- fork-seed provenance containing only model-visible history/checkpoint data;
- inherited transcript segments marked as inherited, without copying them into
  a second writable event log.

Required behavior:

- first supported scope is a complete fork at an exact terminal run boundary;
- reject active, partial, missing, corrupt, or non-terminal sources;
- do not copy active attempts, pending approval/input, turn claims, completed
  mutations, or unknown side effects;
- parent and child continue, cancel, and queue controls independently;
- delete/cleanup cannot silently orphan lineage evidence.

Required UI states after CDH lands:

- tree loading, complete, partial, conflict, depth-limited, source unavailable;
- fork confirming, creating, created, retrying, idempotent replay, failed;
- inherited versus owned transcript provenance.

Required final scenario:

Fork one real terminal session, restart the API, continue parent and child with
different prompts, prove new runtime identity/context inheritance, and prove no
previous file mutation or tool effect is replayed.

### 5.3 Session model defaults and reasoning

The current API-persisted provider/model selection is global. UI-V2 may expose
that truth as a quick **global next-run default**. CDH must add a revisioned
session-scoped run-default contract before the control can claim per-session
scope.

Required session resource:

```text
product_session_id
revision
provider_profile_id
model
reasoning_effort: provider_default | low | medium | high
```

Required capability response:

```text
model_id
context_window_tokens?
reasoning.supported
reasoning.efforts[]
reasoning.unavailable_reason?
```

Required behavior:

- writes use revision CAS;
- product job preparation resolves profile/key-env/model/reasoning on the
  server rather than accepting a browser-reconstructed provider config;
- unsupported efforts are disabled with a reason and are not silently ignored;
- hidden chain-of-thought and provider `ThinkingDelta` never reach Web state.

Required UI states after CDH lands:

- loaded, saving, saved, conflict, stale, unsupported, provider unavailable;
- exact global versus session scope copy;
- reasoning `provider default` rather than a fabricated level when unsupported.

Required final scenario:

Give Sessions A and B different server defaults, refresh/restart, run both, and
assert their exact resolved provider/model/reasoning snapshots without browser
configuration authority.

### 5.4 Context limits, pricing, usage, and cost

UI-V2 uses current canonical usage and approximate prompt metadata. It must not
derive a context percentage without a model limit or derive currency from token
counts.

CDH must provide:

- model context-window capability when known;
- versioned server-owned price metadata;
- immutable per-run provider/model/rate/currency/source snapshot;
- a bounded durable metrics read model if transcript/report access is
  insufficient for long histories.

Cost states are exactly:

- `estimated` when usage and a persisted price snapshot exist;
- `zero/local` only when the server explicitly classifies the model that way;
- `unavailable` when price metadata is missing.

Required final scenario:

Prove live/restored usage equality, no replay double count, known and unknown
context limits, configured cost arithmetic, explicit local zero, unavailable
external pricing, and historical immutability after price changes.

### 5.5 Workspace files, Tool Artifacts, Diffs, and images

CDH must provide separate bounded metadata/text/content contracts. Requests use
`ProductWorkspaceId` plus a normalized relative path, or an opaque artifact ID;
they never accept an absolute host path from the browser.

Required workspace behavior:

- server resolves the exact ProductStore workspace root;
- bounded pagination, depth, entry count, path/name bytes, file bytes/lines,
  range, and timeout;
- reject traversal, invalid encoding, symlink/junction escape, post-open
  identity change, special files, denied internal state, and secret-shaped
  paths;
- cross-workspace ID/path substitution fails closed.

Required Tool Artifact behavior:

- opaque ID, safe name, MIME, size, hash, disposition, source run/call, and
  preview capability;
- authenticated same-origin preview/download with `nosniff`, range limits, and
  safe `Content-Disposition`;
- additive canonical tool result/report references;
- restart, repair, cleanup, and missing-content states.

Required image behavior:

- validated raster formats with byte and pixel limits;
- SVG, HTML, unknown binary, and active content never execute inline;
- no automatic remote Markdown image fetch;
- accessible loading, ready, blocked, missing, truncated, and failed states.

Required final scenario:

Use a real built-in tool to create a file mutation, large Diff, and raster
artifact. Inspect them after refresh/restart, cross-check IDs/hash/MIME/report,
and pass traversal, symlink, cross-workspace, secret, oversize, SVG/HTML, range,
and unauthorized negative tests.

### 5.6 Complete evidence export

CDH must keep the current `rove.session.catalog` export distinct and add an
idempotent server export job with preparing, ready, partially redacted, expired,
and failed states.

The evidence bundle requires:

- versioned manifest and provenance;
- workspace/session identity without raw local root by default;
- lineage and inherited-prefix provenance;
- ordered transcript and typed partial reasons;
- exact product/runtime bindings and canonical references;
- context, usage, and immutable cost snapshots;
- tool invocation/result, mutations, Diffs, and artifact manifest;
- explicitly requested allowlisted artifact bytes;
- deterministic redaction/omission report.

Required final scenario:

Generate a bundle from a real forked session after API restart, then cross-check
every included ID and total against transcript, reports, metrics, lineage, and
artifact metadata. Prove idempotency, authentication, TTL, size failure, archive
path safety, and secret/key/token/root redaction.

### 5.7 Desktop dual host

CDH owns a real `apps/desktop` host and host capability adapter after the shared
Web product contracts stabilize. Desktop reuses the same product transport,
IDs, canonical events, state models, and product components.

Desktop-only capabilities may include native folder selection, reveal/open,
secure secret references, notifications, window/tray, update, packaging, and
API process supervision. They must be capability-detected and show an explicit
unavailable state on Web; they do not fork routes, product state, or execution
lifecycle.

CDH must not implement a private embedded Agent loop, separate Steer/Follow-up
queue, separate fork model, or Desktop-only transcript.

---

## 6. Current Worktree Architecture

### 6.1 One state owner and one selected presentation mode

The production composition is:

```text
M1MigrationGate
  -> ServerProductApp                         # current hooks/API authority, once
     -> shared production component tree
        -> data-ui-version=v2                 # Ice Steel, production default
        or data-ui-version=v1                 # legacy stylesheet compatibility
```

The presentation mode is selected server-side by deployment configuration. It
is not a browser preference, URL switch, or Mock state, so there is one route
sync, one continuity owner, one reducer, one restore, and one focused
`EventSource`.

The `v1` mode is deliberately described narrowly: it disables the scoped V2
stylesheet and retains the legacy product stylesheet, but the shared
`Transcript`, `Composer`, `RunInspector`, `WorkspaceTree`, and shell markup are
still the branch versions. It is an emergency visual-compatibility mode, not an
exact historical presenter or behavioral rollback. Exact application rollback
means redeploying the previous known-good Web build; neither path changes API
authority or persisted product state.

No new capability-handshake endpoint may be added here. Current API absence is
represented through compile-time/current-client knowledge and honest UI
omission or read-only unavailable facts. Later CDH work can introduce a typed
server capability contract.

### 6.2 Canonical V2 read models

Timeline identity remains based on current facts:

```text
product_session_id
  + run_ordinal
  + runtime_run_id
  + event_seq
  + canonical correlation identity
```

There is no server timestamp in current `JobStreamEvent`, so UI-V2 does not
invent one. The timeline metadata column uses run ordinal, event sequence, and
canonical event name where available.

Tool groups key on `{runtime_run_id, call_id}`. A start, approval,
completion/failure, mutation, and execution metadata update one group while its
earliest canonical position stays fixed. `ToolCallView` or its replacement must
retain:

- args and tool-use identity;
- approval reason/state;
- complete output or explicit truncation;
- mutation paths, operation, and Diff;
- execution status/risk/read-only/affected-path/workspace-change metadata;
- failure metadata;
- exact run, ordinal, event sequence, and call identity.

Structured token/context facts are keyed and deduplicated by the same canonical
run/event identity. Replay cannot double-count usage.

### 6.3 Rich content boundary

The renderer uses a structured Markdown AST, not ad hoc string replacement.
Raw HTML and unsafe URL schemes are disabled. GFM tables/task lists are
supported. Code highlighting, Mermaid, and Diff parsing are lazy so ordinary
streaming text stays responsive.

Unified Diffs are parsed structurally. Code and Diff content is inert text.
Mermaid uses strict sandboxing, no scripts, external resources, click
callbacks, or unsafe links, and always has an accessible source fallback.

Because the current API has no safe image endpoint, Markdown image nodes do not
issue network requests. They render a clear blocked/unavailable representation
with safe alt/source text. Real local/artifact rendering is enabled only by CDH.

Any new npm dependency is added only for an actual renderer requirement and is
reviewed for lockfile change, maintenance, bundle size, CSP, sanitization, and
streaming behavior.

---

## 7. Current Worktree Delivery Phases

### W0 - Characterize production contracts and scope presentation

Deliverables:

1. Lock Web characterization tests for exact product turns, canonical restore,
   run/sequence order, replay dedup, failed-cancel observation, ambiguous start,
   workspace Memory selection, Preferences CAS, and migration ordering.
2. Keep `ServerProductApp` as the single orchestration and state owner while
   adapting its existing production component leaves.
3. Scope all new Ice Steel tokens and component rules under the server-selected
   `data-ui-version="v2"` boundary.
4. Make V2 the production default at W5; retain a server-only `v1` legacy-style
   compatibility mode and the previous known-good build as the exact rollback.
5. Add production import-boundary tests that reject preview imports and Mock
   fixtures.

Exit criteria:

- current production behavior is unchanged through live local API acceptance;
- only one continuity hook/reducer/EventSource is mounted;
- no file outside the allowed Web/docs scope changes;
- the previous known-good Web build remains the exact rollback point, and the
  branch-local `v1` compatibility mode remains legible and operable.

### W1 - Ice Steel shell, navigation, Settings, and responsive states

Deliverables:

1. Scoped approved V2 light/dark tokens and fixed shell geometry.
2. Product Bar, workspace rail/drawer, main track, Evidence Inspector
   rail/sheet, composer frame, and Settings shell.
3. Production loading, empty, partial, error, conflict, running, attention,
   canceled, failed, and complete states.
4. Existing Workspace/Session/provider/Preferences/Memory/runtime-health and
   Settings actions wired to current clients.
5. Mobile focus trap/return, inert overlays, long-label truncation, keyboard
   order, live announcements, and reduced-motion states.

Exit criteria:

- every visible command performs a current real API action or a local pure
  presentation action such as disclosure/theme rendering;
- no Steer, Follow-up, fork, artifact, file-open, full-export, reasoning, or
  Desktop action appears;
- all current C0-C4 routes and Settings workflows pass against live API;
- real API screenshots pass at `1440x900`, `390x844`, and `375x812` in both
  themes, including keyboard/reduced-motion states;
- no overflow, overlap, clipped focus, gradient/glow/glass, raw out-of-token
  color, viewport font scaling, or `transition: all` is introduced.

### W2 - Ordered rich transcript and grouped tools

Deliverables:

1. Preserve full current tool/result/mutation/metadata payloads in Web state.
2. Render one ordered Trace Rail with run boundaries, partial notices, canonical
   selection, and anchor-preserving backfill/windowing.
3. Group invocation, approval, result/failure, scope, mutation, and Diff by
   run-qualified call identity without moving canonical interactions.
4. Add safe Markdown/GFM, inline/fenced code, lazy syntax highlighting, strict
   Mermaid, and structured unified-Diff rendering.
5. Share selected item identity with the Evidence Inspector.
6. Batch streamed deltas and preserve user scroll position; show a return-to-
   latest action instead of stealing scroll.
7. Render image references as blocked/unavailable without a network fetch.

Exit criteria:

- a real Fake-provider/tool session proves interleaving before and after
  refresh/SSE replay with no duplicates;
- actual built-in tool mutation Diffs render and agree with canonical/report
  evidence;
- approval and input remain actionable at the correct canonical position;
- XSS, raw HTML, unsafe link/image, Mermaid injection, malformed Diff,
  oversized block, lazy-render failure, and replay tests pass;
- long transcript windowing preserves order, focus, and selected anchor.

### W3 - Composer, quick global model control, metrics, and current evidence

Deliverables:

1. Keep the composer disabled for a real active/attention session where the API
   cannot accept a normal turn; explain the state without presenting a dead
   Steer/Follow-up control.
2. Preserve current Send, Stop, approval, input, cancel-failure observation,
   restore, and ambiguous-submit behavior.
3. Add a compact provider/model control that writes the existing
   API-authoritative global `provider_selection` using current Preferences CAS.
4. Label its scope exactly: global default for the next product run, not a
   session-scoped override.
5. Retain and display per-run prompt/completion/cached/total usage and
   approximate prompt-build/context facts from current canonical events.
6. Display cost as unavailable, with no derived currency or misleading zero.
7. Surface plan, continuity, canonical IDs, mutation/Diff, partial-history, and
   safe runtime status in the Inspector.

Exit criteria:

- changing the quick model updates the real Preferences revision, is reflected
  in Settings, survives refresh, and affects the next real job request;
- a CAS conflict leaves the server value authoritative and presents a typed
  reload/retry path;
- usage/context survive transcript restore and replay without double count;
- running, attention, cancel failure, uncertain start, approval, and input
  states never show fake success;
- there is no reasoning selector, fake cost, queue state, or private lifecycle.

### W4 - Current management completeness and truthful export

Deliverables:

1. Complete all nine Settings sections in the V2 visual system using their
   existing real clients.
2. Preserve provider CRUD/test/model list, approval defaults, max steps,
   workspace pin/remove, session rename/delete, Memory read/delete, runtime
   health, keyboard shortcuts, theme, and About state.
3. Retain the current safe session export and label it
   **Catalog metadata export** everywhere.
4. Do not expose full evidence, artifact inclusion, lineage, or cost options
   that the API cannot fulfill.
5. Keep migration and deep-route recovery ahead of ProductApp catalog state.

Exit criteria:

- no Settings section contains a placeholder, dead action, Mock result, or
  optimistic success;
- every existing action passes component, mocked fault/race, and relevant live
  API tests;
- export content still excludes roots, runtime IDs, transcript, provider
  config, and secrets exactly as its current versioned contract specifies;
- mobile Settings labels scroll rather than clip and all save/conflict/error
  states are visible.

### W5 - Current-scope integration and merge readiness

Deliverables:

1. Run aggregate Rust regression gates without changing Rust sources.
2. Run complete Web unit/type/build/e2e gates.
3. Expand live `local-full` ProductApp coverage for W1-W4 current API features.
4. Run the production visual/accessibility matrix on real API state.
5. Prove preview/production import and request separation.
6. Update Web/current-state documentation only for behavior actually merged.
7. Make V2 the production presentation after current-scope gates pass,
   retaining the `v1` legacy-style compatibility mode for one release window
   and the previous known-good Web build for exact rollback.
8. Record all CDH rows as assigned and not yet implemented; do not mark final
   Pi acceptance complete.

Exit criteria:

- every UI-V2 matrix row is backed by current real API evidence;
- existing approval/input/cancel/resume/migration/Settings/Memory/provider and
  exact-continuity flows remain green;
- V2 production contains no preview imports or Mock authority;
- every visible action has a real implementation and every unavailable
  capability has no action affordance;
- the branch is merge-ready, but full Pi/dual-host production acceptance remains
  explicitly pending CDH and FINAL.

---

## 8. Worktree And File Ownership

### 8.1 Current worktree ownership

`.worktrees/product-ui-v2-production` owns only:

- `apps/web/shell/**` production composition and presentation-mode selection;
- new `apps/web/product-v2/**` production components/read models;
- `apps/web/chat/**`, `apps/web/inspector/**`, `apps/web/settings/**`, and
  `apps/web/sidebar/**` where current components are adapted or retired;
- `apps/web/state/**`, `apps/web/api/**`, `apps/web/lib/**`, and
  `apps/web/product/**` only for consuming and presenting the current API;
- `apps/web/styles/**`, Web-only package/lockfile changes needed by renderers;
- `apps/web/tests/**`, Web README, and this implementation plan;
- current runtime documentation only when required to describe a merged Web
  behavior, without changing a Rust/API contract claim.

### 8.2 Explicitly prohibited in this worktree

This worktree does not edit:

- `runtime/**`;
- `apps/api/**`;
- ProductStore contracts, repository, or schema;
- generated OpenAPI or Rust API types;
- `core/**`, `models/**`, or `apps/bootstrap/**`;
- Rust integration tests to make a missing API look implemented;
- `apps/web/app/dev/product-ui-v2/**` as a source for production state or
  behavior.

If current Web types do not match a field already emitted by the current API,
the Web parser/type may be corrected to the verified existing wire shape. That
is client conformance, not permission to change the server contract.

### 8.3 Follow-up unified owner

The subsequent program uses a coordinator worktree such as:

```text
.worktrees/control-desktop-dual-host
branch: feature/control-desktop-dual-host
```

It owns all CDH rows together:

- durable control receipts and canonical application facts;
- session lineage and runtime fork;
- session-scoped run defaults and reasoning capabilities;
- context limits, pricing, durable cost snapshots, and any required metric
  aggregate;
- workspace files, Tool Artifacts, raster images, and large Diffs;
- complete evidence export;
- typed host capabilities and real `apps/desktop` implementation;
- narrow Product UI V2 adapter changes needed to consume those new contracts;
- combined Web/Desktop real end-to-end acceptance.

The program may split internal workers only after one coordinator-owned
contract foundation is sealed. Those internal splits remain one unified owner
and are not separate unassigned follow-ups.

### 8.4 Merge order

```text
this plan
  -> W0 presentation scope
  -> W1 shell/settings
  -> W2 rich ordered timeline
  -> W3 current composer/model/metrics
  -> W4 current management/export
  -> W5 current-scope live acceptance
  -> merge Product UI V2 production interface
  -> CDH contract + Web adapter + Desktop dual-host implementation
  -> FINAL combined real end-to-end acceptance
```

The CDH coordinator starts from the merged UI-V2 result, not from the Mock
preview or this planning baseline.

---

## 9. ProductApp Replacement Order

The replacement order is fixed:

1. Characterize C4 and retain `ServerProductApp` as the single state owner.
2. Add server-side presentation-mode selection with strictly scoped V2 styles;
   treat `v1` only as a legacy-style compatibility mode.
3. Mount V2 Product Bar, shell tracks, Settings, drawers, and existing state
   views over the shared production view model.
4. Replace transcript and Inspector presentation with W2 ordered rich read
   models while retaining the canonical reducer/controller.
5. Replace composer presentation while preserving current Send/Stop/approval/
   input/restore semantics; do not add Steer/Follow-up affordances.
6. Add the real global quick model control and structured usage/context facts.
7. Complete current Settings and truthful catalog export.
8. Pass W5 against the real API, keep V2 as the production default, retain the
   bounded legacy-style mode, and preserve the previous build for exact
   rollback.
9. After CDH merges, enable its controls/tree/resources/export/host features
   only from real server contracts and run FINAL.

At no point are two product state owners mounted together. At no point is a
Mock fixture used to fill a missing API.

---

## 10. Verification Strategy

### 10.1 Current worktree gates

Run focused Web tests first, then:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

From `apps/web/`:

```powershell
pnpm test
pnpm typecheck
pnpm build
pnpm test:e2e
```

Live current API acceptance:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/integration-smoke.ps1
```

Mock-backed Playwright remains valid for network disconnects, delayed
responses, stale SSE, cancellation failure, CAS conflict, malformed payload,
and session-switch races. It does not close a real integration row by itself.

### 10.2 Current real API scenario

W5 must record exact IDs through this scenario:

1. Start an isolated Fake-provider API/ProductStore/workspace and open the real
   production route with V2 selected server-side.
2. Prove M1 migration/no-migration behavior before catalog reads.
3. Create Workspace W and interleaved Sessions A/B.
4. In A, run assistant Markdown/code/Mermaid and a built-in tool that emits a
   mutation/Diff, approval, and input request where possible.
5. Verify exact ordered/grouped facts, submit approval/input, and exercise Stop.
6. Force one cancel request failure and prove observation remains attached.
7. Refresh A and prove identical order, structured usage/context, and no replay
   duplicate.
8. Change the quick global provider/model selection through the composer,
   verify Preferences revision/Settings synchronization, and use it for the next
   turn.
9. Switch A/B repeatedly and prove no transcript/run state contamination.
10. Exercise all current Settings/Memory/provider/session/catalog-export paths
    and deep routes.
11. Run light/dark desktop/mobile, keyboard, focus, reduced-motion, overflow,
    console, and nonblank-pixel assertions.

The scenario must assert `product_session_id`, job/run IDs, run ordinals, event
sequences, call IDs, Preferences revision, and transcript/report facts rather
than selecting a guessed latest run.

### 10.3 Final combined scenario

FINAL is owned by CDH after UI-V2 merges. It extends the W5 scenario with:

- real Steer and ordered Follow-up receipts across reload/restart;
- session tree and terminal-boundary fork with divergent continuation;
- session-scoped model/reasoning defaults;
- known/unknown context limits and durable usage/cost snapshots;
- real workspace file, artifact, raster image, and large-Diff viewing;
- complete evidence export and redaction verification;
- Web/Desktop host parity over identical product/runtime identities.

Only this combined scenario can mark the full Pi capability matrix and Product
UI V2 productionization `Met`.

### 10.4 Visual and accessibility matrix

Use real API state for:

| Surface | Viewport | Themes | Required current state |
|---|---:|---|---|
| Conversation | `1440x900` | light/dark | active stream, rich assistant turn, grouped tool, approval, usage |
| Conversation | `390x844`, `375x812` | light/dark | active run, stable composer, workspace drawer, Evidence sheet |
| Settings | desktop/mobile | light/dark | provider, approval, Memory, conflict/error, catalog export |
| Restore | desktop/mobile | either | complete, partial, failed/retry, stale-session rejection |
| Any active state | desktop/mobile | either | reduced motion and keyboard-only |

Automation asserts nonblank pixels, no page/console error, document width
containment, non-overlapping tracks, contained overlays, stable geometry,
visible focus, correct focus return, readable labels, and no hidden action under
another layer.

### 10.5 Mock and dead-action gates

W0 and W5 add explicit checks that:

- production modules contain no import from `app/dev/product-ui-v2`;
- preview fixtures/types are not re-exported by production modules;
- the production route calls real `/api/product/**` and `/api/jobs/**` for all
  visible capabilities;
- the preview still performs zero `/api` requests in its isolated test;
- no new product authority is persisted in localStorage, IndexedDB, or a
  service worker;
- real-API Playwright has no request interception;
- every button/link/menu item has a tested real command;
- missing CDH capabilities have no action affordance in UI-V2;
- request failure, conflict, timeout, and ambiguity never become visual success.

---

## 11. Compatibility, Migration, And Rollback

### 11.1 Compatibility

- `M1MigrationGate` stays ahead of all product catalog reads and preserves its
  exact retry/deep-route mapping behavior.
- ProductStore schema and runtime artifacts are unchanged by this worktree.
- Existing API URLs, payloads, SSE event shapes, and OpenAPI are unchanged.
- Pre-V2 API contracts and persisted product state remain compatible. The
  branch-local `v1` mode preserves legacy styling only; it does not promise the
  old component tree.
- Direct `/jobs`, `/dev/workbench`, and the inert preview remain separate from
  the ProductApp presentation migration.
- Current safe export keeps its schema/version and exact catalog-only meaning.
- Quick model changes the current global provider selection only; it does not
  silently migrate or create per-session state.

### 11.2 Presentation and deployment rollback

Exact rollback redeploys the previous known-good Web build. As a faster
legibility fallback, setting `ROVE_PRODUCT_UI_VERSION=v1` server-side disables
the scoped V2 stylesheet while keeping this branch's shared components. The
latter is not an exact presenter rollback.

Neither rollback path clears browser storage, rewrites ProductStore, alters
runtime bindings, or creates a second event lifecycle. The rollback drill
proves:

- exact session continuation remains correct;
- approval/input/cancel/restore/Settings remain usable;
- migration does not replay;
- returning to the V2 build/mode restores the same API-authoritative state;
- no V2-only browser state needs migration.

Because this worktree adds no backend contract, backend/database rollback is
out of scope here. CDH must define reader-first event/schema rollout and its own
backend rollback floor before emitting new canonical facts.

---

## 12. Security Review

### 12.1 Current Web work

- Bound Markdown, code, Diff, tool output, labels, and rendered collection
  sizes before DOM expansion.
- Parse Markdown and Diff structurally; disable raw HTML and unsafe URL schemes.
- Code/Diff are inert text; Mermaid is strict/sandboxed with no script,
  external fetch, callback, or unsafe link.
- Do not automatically load remote images. Do not create a browser proxy around
  the missing file/artifact API.
- Keep canonical item identity attached to approvals/inputs so virtualization
  cannot submit against the wrong call.
- Preserve one focused EventSource and stale generation rejection.
- Keep raw provider keys out of Web state; only current safe profile/key-env
  references are rendered.
- Preserve existing Next.js bearer injection, CORS, rate-limit, and API error
  behavior by using current proxy/client paths.
- Treat cost as unavailable, approximate tokens as approximate, and blocked
  image/file/artifact states as unavailable rather than successful.

### 12.2 CDH handoff controls

The later program must additionally enforce:

- idempotency and CAS for controls, fork, defaults, artifacts, and export;
- bounded bodies, queues, files, artifacts, ranges, archives, and pagination;
- workspace paths resolved only from `ProductWorkspaceId`;
- traversal/symlink/junction/special-file/secret-path rejection;
- MIME allowlists, `nosniff`, safe disposition, and no inline SVG/HTML;
- no replay of applied controls, completed mutations, or unknown side effects;
- export redaction/provenance and immutable price/cost provenance;
- identical auth/CORS/rate-limit boundaries for new Web/Desktop transport.

---

## 13. Documentation And Completion

UI-V2 completion means:

1. W0-W5 are implemented in this worktree without prohibited backend changes.
2. Every UI-V2 matrix row passes Web and real current API gates.
3. The approved visual solution is implemented without redesign.
4. Existing C0-C4 behavior remains green.
5. Production contains no Mock state, dead action, fake success, or private
   event lifecycle.
6. Every absent API capability is omitted or shown only as an honest read-only
   unavailable fact and remains assigned to CDH.
7. V2 is merge-ready as the production Web presentation with a bounded legacy
   stylesheet compatibility mode and a documented previous-build rollback.
8. Current documentation describes only behavior that actually merged.

Full Product UI V2 productionization completion means, later:

1. UI-V2 has merged.
2. CDH has implemented every missing contract, Web adapter, and Desktop host
   assigned in Sections 4 and 5.
3. FINAL passes the complete real Web/Desktop end-to-end scenario.
4. Runtime implementation status and acceptance matrix are updated to `Met`
   only from that evidence.
5. Optional external-provider interoperability is claimed only if its explicit
   gate actually ran.

The immediate next action after plan review is W0 in the current worktree. No
Rust/API/ProductStore/OpenAPI work begins here.

---

## 14. Current-Scope Evidence Record

Recorded 2026-07-28 on branch `feature/product-ui-v2-production` in
`.worktrees/product-ui-v2-production`. This record covers the UI-V2 current
scope only; the CDH rows in Sections 4 and 5 remain assigned and not yet
implemented, and full Pi/dual-host acceptance remains pending CDH and FINAL.

### 14.1 Implemented scope

- **W0**: production import-boundary tests
  (`apps/web/product/production-ui-boundary.test.ts`) reject preview imports,
  Mock authority, and affordances for CDH-absent contracts (Steer, Follow-up,
  Fork, Reasoning, file/artifact browse, Desktop). One state owner:
  `M1MigrationGate -> ServerProductApp` composition preserved in
  `apps/web/shell/ProductApp.tsx`; server-side presentation selection via
  `ROVE_PRODUCT_UI_VERSION` in `apps/web/app/(product)/layout.tsx`
  (`v2` default, `v1` legacy-stylesheet compatibility mode).
- **W1**: scoped Ice Steel tokens and component rules under
  `.product-app-frame[data-ui-version="v2"]` in
  `apps/web/styles/product-v2.css` (light/dark palettes, `3/5/8px` radii,
  mobile 44px touch targets, `prefers-reduced-motion` terminal states, no
  gradient/glow/`transition: all`/viewport-scaled type). Shell, workspace
  rail/drawer with focus trap + search, Evidence Inspector rail/sheet with
  focus trap and focus return, inert main track under overlays, mobile scrim.
- **W2**: full wire facts retained in Web state (`ToolCallView.args/output/
  error/mutations/metadata`, `ChatMessage.usage/promptBuild/promptCompaction`,
  run-scoped `runUsage/promptBuild/promptCompaction` in
  `apps/web/lib/rove-state.ts`; `ToolExecutionMetadata` client conformance in
  `apps/web/lib/rove-types.ts` matching `core/src/types.rs`). Ordered Trace
  Rail with run boundaries, canonical event-seq meta column, return-to-latest
  scroll action in `apps/web/chat/Transcript.tsx`. Safe GFM Markdown with raw
  HTML disabled and URL scheme allowlist (`product-v2/RichText.tsx`), lazy
  Prism code with copy state (`product-v2/RichCodeBlock.tsx`), strict
  sandboxed Mermaid with SVG sanitization (`product-v2/MermaidDiagram.tsx`),
  structural unified-Diff rendering with honest synthesized headers for
  headerless canonical mutation Diffs (`product-v2/DiffView.tsx`), and blocked
  image representations without network fetch.
- **W3**: composer preserves Send/Stop/approval/input/ambiguous-submit
  semantics and explains running/attention disabled states without
  Steer/Follow-up affordances (`apps/web/chat/Composer.tsx`). Quick global
  model control writes the real API-persisted `provider_selection` through
  Preferences CAS with explicit global next-run scope labeling
  (`product-v2/QuickModelControl.tsx`). Inspector renders continuity
  identities, per-run prompt/completion/cached/total usage, approximate
  context facts, compaction state, plan, approvals, tools, workspace
  mutations, canonical event trace, and honest `Cost: Unavailable`
  (`apps/web/inspector/RunInspector.tsx`).
- **W4**: all nine Settings sections run in the V2 system over existing real
  clients; session export relabeled **Catalog metadata export** everywhere
  (`apps/web/settings/CatalogSettings.tsx`).
- **W5**: gates below; V2 is the production default with the `v1`
  legacy-stylesheet compatibility mode retained and the previous known-good
  Web build as the exact rollback.

### 14.2 Verification evidence

All run in this worktree on 2026-07-28:

| Gate | Result |
|---|---|
| `cargo fmt --all --check` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | 263 passed, 0 failed |
| `pnpm test` (`apps/web`, vitest) | 27 files, 185 tests passed |
| `pnpm typecheck` (`apps/web`) | pass |
| `pnpm build` (`apps/web`) | pass (all routes compiled) |
| `pnpm test:e2e` (mock-backed Playwright) | 54 passed, 3 skipped |
| `scripts/integration-smoke.ps1` (`local-full`, real Fake-provider API) | 3/3 real-API Playwright scenarios passed |

Live `local-full` run ids recorded by the smoke: `01KYK5VV54MXJN4EKGRFA9WT6B`,
`01KYK5VVSVSGVZ5M532EVB9F61`, `01KYK5VXBNEPJA9A0AJ5K4G10Y`,
`01KYK5VYE0VCFPBARDXFN13E5S`, `01KYK5VZFP16963J6SS4F8ZCW8`,
`01KYK5W0H80F3GBJCE6QKKBRN1`.

The real-API scenario covered M1 migration before catalog reads, exact A/B
session continuity with refresh and tool approval/cancellation, quick-model
Preferences persistence across reload, Settings surfaces, and the bounded
advanced workbench smoke. The external-provider gate was not run; no external
interoperability claim is made.

### 14.3 Defects corrected during the W0-W5 audit

1. `finalizeAssistantMessage` could stamp the next segment's zeroed run-scoped
   usage onto an already-final assistant message when a completion event was
   duplicated across overlapping restore ranges; the reducer now treats a
   final message with identical content as a replay no-op, preserving omitted
   facts (regression: `transcript-projection.test.ts` "never substitutes
   zero-usage evidence onto a restored earlier draft").
2. Headerless canonical mutation Diffs (e.g. `"+canonical"`) previously fell
   into the unstructured fallback or a fabricated file name; `DiffView` now
   synthesizes an honest unified envelope from real line counts keyed by the
   canonical mutation path and labels the synthesis
   (`product-v2/DiffView.test.tsx`).
3. Mermaid strict rendering hid SVG text labels in browsers; labels render as
   SVG text (`htmlLabels: false`) with `foreignObject` sanitization
   (regression: `polish.spec.ts` "strict Mermaid rendering preserves visible
   SVG text labels").
4. Workspace drawer search was fixed and covered by the mobile focus-trap
   scenario in `polish.spec.ts`.
5. Code-block copy feedback is announced with an auto-clearing live-region
   state (`product-v2/RichCodeBlock.tsx`).

### 14.4 Boundary audit

- `git diff` against the baseline touches only `apps/web/**` and
  `docs/plans/2026-07-28-product-ui-v2-production.md`; `runtime/**`,
  `apps/api/**`, `core/**`, `models/**`, `apps/bootstrap/**`, ProductStore,
  OpenAPI, and Rust tests are unchanged.
- New npm dependencies (`react-markdown`, `remark-gfm`, `mermaid`,
  `parse-diff`, `prism-react-renderer`) exist only for actual renderer
  requirements; lockfile updated by pnpm.
- Production modules contain no `app/dev/product-ui-v2` import, no Mock
  fixture, no localStorage/IndexedDB/service-worker product authority, and no
  request interception in the real-API suite.
- Every visible command performs a current real API action or a pure local
  presentation action; CDH-absent capabilities have no action affordance and
  are shown only as honest read-only unavailable facts (cost, artifacts,
  images, large Diffs).

### 14.5 Not done here (assigned, not unowned)

Every CDH row in Sections 4 and 5 (durable Steer/Follow-up, lineage/fork,
session-scoped defaults and reasoning, context limits/pricing/cost snapshots,
workspace files/artifacts/images/large Diffs, complete evidence export,
Desktop dual host) remains assigned to the Control Capabilities and Desktop
Dual-Host Implementation program. FINAL combined acceptance runs only after
UI-V2 merges and CDH lands.
