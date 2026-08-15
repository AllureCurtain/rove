# Post-Full-Delivery Productization Program

> Status: **A-E and F.1-F.3 implemented on `productization/integration`; F.4/F.5 and G partially complete**
>
> Verified baseline: Checkpoints 0-9 were delivered at `45143a5` and merged
> through PR #30 as `4b740d3`. The machine-generated acceptance report was
> produced from clean code commit `32ba252`: all 11 required checks passed,
> with zero failures and zero required checks unrun. PR #31 merged the
> whitespace-only documentation cleanup as `1b57b36`. Productization integration
> combines runtime intelligence (`579c60f`), Provider configuration (`77f3ecf`),
> and unified conversation delivery (`2b7e798`) plus the cross-workstream schema,
> exact-run attachment, browser, and isolation fixes on
> `productization/integration`. Workstreams A-E and F.1-F.3 are implemented.
> F.4/F.5 remain partial: older-history pagination/windowing and complete TUI
> restart recovery are not implemented. Workstream G has deterministic and
> live local fake-provider evidence, while the external
> Provider, real third-party/official filesystem MCP, Windows ConPTY,
> macOS/Linux package, signing, and installed-Desktop gates remain unverified.
>
> This is the single active productization brief after full-delivery. The dated
> 2026-08-09 documents are current-state corrections, audit evidence,
> historical comparisons, or explicitly deferred-capability records. They are
> not implementation plans and cannot add work to this program.
>
> Product interaction decision (2026-08-10): the user-facing composer has one
> message action. A message submitted while a run is active is durably queued
> for the next turn by default. The user may promote that same queued message to
> an immediate intervention, which the Harness applies to the current run at the
> next protocol-safe boundary. `Steer` and `Follow-up` may remain compatibility
> terms in existing APIs and migrations, but they are not user-selected modes.

## 0. Implementation Authority

This document is the only implementation contract for the productization work
that follows the verified full-delivery program. All target
behavior, in-scope deliverables, compatibility requirements, exclusions, and
acceptance evidence for this round must be stated here.

The authority split is strict:

| Source | Permitted use in this program |
|---|---|
| Current code, tests, schemas, and `docs/runtime/` | Verify the actual baseline and detect contradictions |
| This document | Define what the next implementation must deliver |
| 2026-08-09 and earlier analysis/design/plan documents | Explain an audited defect, correction, rejected option, historical decision, or deferred boundary only |

A heading such as `Proposed Design`, `Acceptance Criteria`, `P0`, or `P1` in an
older document does not create a task. A requirement from an older document is
in scope only when it is restated in this file. If an older document conflicts
with this file, current runtime facts govern the baseline and this file governs
the future target.

Do not create another implementation plan, gap register, or checkpoint brief
for the same scope. Amend this document when the product contract changes. A
supporting source may be opened to inspect evidence while implementing a named
workstream, but future agents must not execute a supporting source as a plan.

## 1. Mission

Turn the completed local-first runtime into a product that a new developer can
install, configure, trust, use on a real repository, recover after failures, and
evaluate without reading the implementation.

The goal is not another architecture expansion. The goal is dependable value in
the first ten minutes and dependable behavior over a long coding session:

```text
install -> choose workspace -> configure provider -> ask for work
  -> inspect -> change -> verify -> review evidence -> resume or retry
```

The program keeps one shared Engine/kernel, one ToolRegistry and Executor path,
one canonical event lifecycle, one durable artifact authority, and one set of
Runtime-owned safety decisions across CLI, API, Web, TUI, and Desktop.

### 1.1 How to start future work

This file is the only productization entry point and the only future-task
document. A future implementation conversation should be asked to read
repository `AGENTS.md`, the current `docs/runtime/` source of truth, and this
document, then execute the complete in-scope program. Section 7 sources are
optional audit inputs, not additional specifications.

Do not ask an implementation agent to "complete" a dated supporting document.
Do not infer extra requirements from it, and do not recreate a gap register or
checkpoint plan unless the user materially changes this product contract.

## 2. Verified Completion Baseline

Full-delivery is present on `main` and verified by code, tests, CI, the
machine-generated acceptance report, and the PR #30/#31 merge history:

| Area | Verified result |
|---|---|
| Execution | One shared Agent kernel for embedded and durable execution |
| Lifecycle | Rule-first decisions, bounded ambiguity evaluation, Finalizer, budgets, reconciliation, resume |
| Protocol | Shared MCP dispatcher with stdio, legacy SSE, and Streamable HTTP |
| Results | Rich normalized result envelope, bounded content blocks, ArtifactRef, durable Tool Artifacts |
| Agent configuration | Versioned AgentDefinition, immutable runtime profile, trusted instruction discovery, typed procedures |
| Evaluation | OnCall reference fixtures, deterministic oracles, safety gates, evidence packages |
| Product host | Shared Web/API/Engine/ProductStore plus verified Desktop host and packaging evidence |

Optional external-provider, official filesystem MCP, macOS/Linux packaging,
manual installer, and complete installed-Desktop interaction evidence remain
unverified. No downstream task may turn those explicit boundaries into passing
claims or treat a design document as proof of implementation.

### 2.1 Productization integration record

| Workstream | Implementation status | Evidence boundary |
|---|---|---|
| A. Baseline and documentation seal | Implemented in the integration change | Root/current runtime documents, acceptance/release matrices, and this plan agree on implemented versus unverified behavior. Historical dated audits remain supporting inputs. |
| B. Agent effectiveness and tool-call recovery | Implemented | Native-first tool calls, explicit compatibility parsing, typed recovery, bounded prompt metadata, and negative safety tests. |
| C. Repository understanding and retrieval quality | Implemented | Ignore-aware deterministic traversal/search/glob, typed bounded outcomes, and manifest-only repository maps. |
| D. Context economy and result history | Implemented | Current results stay inline; eligible older duplicates use deterministic durable Artifact references with explicit resolution. |
| E. First-run configuration and Provider onboarding | Implemented | User catalog schema v1, authority-aware load/write, env/file/keyring references, API/Web CRUD and probes, migration, per-turn CLI assembly, and TUI `/model`. External interoperability is not implied. |
| F. Unified conversation and product experience | Partially implemented | F.1-F.3 provide one durable message identity and six delivery states across Runtime, ProductStore/API/SSE, Web, and TUI; ProductStore v13 reconciles both parallel v12 layouts. F.4 older-history pagination/windowing and F.5 complete TUI restart recovery remain open. |
| G. Real-world evaluation and release confidence | Partially complete | Deterministic Rust/Web/TUI coverage and five live local fake-provider browser scenarios pass. Credentialed Provider, real MCP, Windows ConPTY, non-Windows packaging, signing, installed-Desktop, and broader soak gates remain unverified. |

The latest integration verification on 2026-08-14 recorded:

| Gate | Result |
|---|---|
| Rust formatting, clippy, and workspace tests | Passed before the final integration fixes. The final sensitive-header change separately passed formatting and its focused unit test; full clippy/workspace tests were not repeated. |
| `pnpm test` | 36 files and 241 tests passed. |
| `pnpm typecheck` / `pnpm build` | Passed. |
| `pnpm test:e2e` | 56 deterministic browser tests passed; five live-API cases were correctly skipped without the environment gate. |
| `local-full` | 5/5 live local fake-provider cases passed: migration; A/B continuity and interactions; unified message promote/revoke; completed-session Fork with independent child continuation; bounded workbench smoke. |

The successful `local-full` artifacts were kept outside the repository under
`%TEMP%\rove-productization-integration-9\artifacts`. They are local evidence,
not committed release artifacts and not external-provider evidence.

## 3. Product Definition Of Done

The productization program is complete when all of the following are true:

1. A new user can select a workspace, select a supported provider profile, run a
   read-only task, understand what happened, and see the evidence without
   editing TOML by hand.
2. A real provider can perform native tool calls, recover from invalid tool
   arguments, and finish a small repository task without JSON-text tool-call
   confusion.
3. Search and file inspection do not waste bounded budgets on generated output,
   hidden build trees, unstable traversal order, or avoidable re-reads.
4. Mutations remain approval-gated, workspace-bounded, version-checked, and
   auditable; cancellation and unknown external effects remain visible.
5. Long chats stay responsive, streaming follows the user's reading position,
   internal runtime identifiers stay out of the main product transcript, and
   large tool output remains inspectable through safe artifact projections.
6. The composer has one message path: idle messages start work, messages sent
   during active work are durably queued in order, and a queued message can be
   promoted without duplication for delivery to the current run at the next
   protocol-safe boundary.
7. A failed, cancelled, disconnected, or restarted run has an honest visible
   state and can be resumed without replaying completed side effects.
8. Deterministic local evidence, real-provider evidence, browser evidence,
   package evidence, and security review are reported separately and honestly.
9. The optional full-screen TUI presents the same durable message states and
   safety outcomes through terminal-native interactions without creating a
   private queue, lifecycle, permission path, or persistence format.

## 4. Workstreams

These workstreams may proceed in parallel after the baseline seal. The stated
dependencies are contract dependencies, not a requirement to conduct one long
manual step-by-step session.

### A. Baseline and documentation seal

Deliverables:

- record the final full-delivery commit and all required gate exits;
- reconcile `AGENTS.md`, README, `docs/runtime/`, acceptance/status matrices,
  release readiness, and active plans;
- mark the 2026-08-09 documents as non-authoritative current-state corrections,
  audit inputs, historical comparisons, or deferred-capability records;
- classify external provider, real MCP, Windows PTY, Desktop signing, and other
  unavailable gates as unverified rather than passed;
- produce a secret-free evidence index with provenance and clean-tree status.

This workstream is a prerequisite for every public completion claim, but it does
not block local implementation work that is explicitly marked proposed.

### B. Agent effectiveness and tool-call recovery

Use the completed AgentDefinition/profile/instruction/procedure authority. Do
not introduce a second prompt configuration system.

Deliverables:

- native tool-call first behavior for providers that advertise structured tool
  calls;
- compatibility text parsing restricted to providers that require it;
- malformed text/tool calls become typed recoverable failures, never silent
  terminal answers;
- deterministic schema errors include the field, expected type, received type,
  and a bounded correction example;
- bounded environment, capability, instruction, procedure, and budget guidance;
- planner guidance focused on verifiable outcomes and realistic step budgets;
- prompt/profile hashes and resume identity remain stable when inputs are stable.

The work must keep policy, instructions, memory, procedures, retrieval, and tool
permissions as distinct authorities. Repository-authored text cannot grant a
capability or bypass approval.

Required verification:

- a native-tool-use provider path emits no JSON-text tool actions, while a
  compatibility-only provider retains the minimum explicitly enabled fallback;
- malformed compatibility output cannot become a successful final answer and
  the `regex`-as-string schema regression receives a deterministic correction;
- prompt-build metadata reports bounded component and total byte counts without
  leaking raw paths or secrets;
- unchanged profile, workspace, capability snapshot, procedure selection, and
  tool catalog produce stable prompt/profile/cache identity across turns;
- restricted workspaces contribute no unauthorized project instruction or
  procedure, covered by negative tests;
- the same bounded repository task is measured before/after with deterministic
  fake-provider evidence and a separately classified native-provider result.

This workstream does not add personas, output-style profiles, user-authored
prompt plugins, another reminder/instruction channel, or per-model prompt forks
beyond the native-versus-compatibility tool-call contract.

### C. Repository understanding and retrieval quality

Deliverables:

- `.gitignore` and `.ignore` aware traversal for search, glob, and recursive
  listing, without weakening workspace boundaries;
- deterministic lexical traversal and explicit scanned-file/match/output
  truncation facts;
- bounded context lines for search matches with honest byte accounting;
- maintained glob semantics for recursive patterns, braces, and classes, with
  traversal and symlink/reparse tests;
- safe treatment of binary files, hidden files, ignored files, and sensitive
  paths;
- a bounded repository map derived from manifests and verified documentation,
  available on demand or through a content-addressed cache rather than injected
  into every model turn.

No vector database, embedding index, language server, auto-downloaded binary, or
semantic call graph is part of this workstream. Do not shell out to an optional
system `rg`/`fd` binary for core behavior; deterministic local behavior cannot
depend on what happens to be installed on one machine.

Required verification:

- searches still find real source files when `target/`, `node_modules/`, and
  `.next/` exist, without exhausting scan limits on generated output;
- repeated traversal has identical ordering and truncation metadata;
- surrounding context coalesces overlapping ranges and charges every returned
  byte against the output bound;
- glob tests cover `**/*.rs`, braces, character classes, path escape, symlink,
  and Windows reparse boundaries;
- ignored-file opt-in cannot bypass workspace or sensitive-path policy;
- binary, hidden, ignored, missing, and oversized inputs return typed bounded
  outcomes rather than looking like an empty successful search;
- repository-map output contains only manifest or verified-documentation facts,
  truncates explicitly, and reuses the same digest for unchanged inputs.

### D. Context economy and result history

This workstream builds on the completed `ToolOutputEnvelope`, `ToolArtifactRef`,
and durable Tool Artifact store. It must not create a parallel Observation
authority.

Deliverables:

- keep the current tool result available to the model within bounded limits;
- use deterministic summaries and content-addressed references for repeated or
  older history entries;
- retain full eligible payloads in the canonical artifact store with quota,
  redaction, MIME, sensitivity, and retention rules;
- preserve observation/version preconditions for coding mutations;
- make artifact/reference resolution explicit after resume and after cleanup;
- ensure provider history projection remains valid for every supported wire
  protocol;
- verify that trace and audit projections contain facts, hashes, status, and
  references without assuming that trace is an unbounded raw-output archive.

The reference form must never leave the model unable to access the content it
needs, and an evicted payload must fail loudly if the source version no longer
matches.

Required verification:

- later-turn prompt bytes decrease measurably for repeated reads without hiding
  the current tool result from the model;
- repeated eligible payloads reuse canonical artifact content while retaining
  each call's provenance;
- trace and reports retain facts, hashes, status, and artifact lineage without
  pretending to contain every payload byte;
- interruption/resume resolves every retained reference or returns an explicit
  missing/expired state;
- stale observation/version mutations remain rejected exactly as before;
- replaying the same recorded run produces byte-identical assembled history and
  valid provider projections for every supported wire protocol.

This workstream does not add model-generated summaries, similarity/embedding
deduplication, cross-run Observation sharing, a second artifact database, a new
approval path, or a private event lifecycle.

### E. First-run configuration and provider onboarding

Deliverables:

- provider preset catalog for supported protocol families and common gateways;
- base URL, protocol, model, documentation, and credential-reference guidance
  filled from the selected preset;
- server-side or Desktop-safe credential handling; raw keys never enter browser
  state, traces, reports, screenshots, or normal logs;
- connection/model test with typed errors for authentication, endpoint,
  protocol, quota, and unsupported-tool failures;
- clear fake-provider mode for no-network development;
- fallback profile behavior and active-profile persistence exposed consistently
  through CLI, API, Web, TUI, and Desktop;
- workspace activation, Project Trust, approval defaults, and capability
  explanations presented before the first mutation-capable run.

Preset data is onboarding assistance, not a replacement for the open provider
registry or a promise that every listed endpoint has been externally verified.

### F. Unified conversation control and Web/Desktop/TUI product experience

This is one cross-layer product task, not a cosmetic Web pass. It includes the
ProductStore/API message lifecycle, Harness delivery boundary, canonical event
projection, Web/Desktop interaction model, terminal-native TUI projection,
recovery, and acceptance evidence. The product should feel like one
conversation even though the Runtime retains strict run, turn, tool, approval,
and evidence boundaries.

#### F.1 One composer and one durable message lifecycle

The user never chooses between `Steer` and `Follow-up` before sending. The
product-domain command is `send message`; server-owned state decides how that
message can be delivered:

| Session state at durable acceptance | Default result |
|---|---|
| Idle with no active turn | Atomically claim and start the next turn |
| Active run | Persist in FIFO order for a successor turn |
| Ambiguous, failed, cancelled, or unknown-effect boundary | Persist without automatic execution and require an explicit recovery decision |

An active-run message appears in the transcript immediately with an honest
queued state. It exposes two semantic actions while still eligible: revoke it,
or request immediate intervention. Exact iconography, styling, and keyboard
binding are deferred to the later visual-design pass; the command contract is
not deferred.

Immediate intervention promotes the same durable message. It must not create a
second control row, duplicate transcript content, or remove the message from the
successor queue before the current run has accepted it. Promotion and terminal
completion race through one atomic compare-and-set decision:

```text
durably queued
  | successful terminal wins              | promotion wins
  v                                       v
claimed for successor turn       intervention requested for current run
                                          |
                                next protocol-safe boundary
                                          v
                              applied to current run exactly once
```

If terminal completion wins the race, the message keeps its original queue
position and runs next. If promotion wins, the Runtime guarantees either one
bounded next model turn containing that message or a typed `needs_attention`
result; it must not acknowledge immediate delivery and later silently drop it.
Multiple queued or promoted messages preserve acceptance order. Queue count,
message bytes, promoted messages per run, and the additional model-turn budget
are bounded. An accepted intervention cannot bypass hard cost, token, step,
approval, or capability limits.

The API should provide one idempotent session-message submission route and one
idempotent promotion route so idle/active classification is server-atomic. The
existing Steer/Follow-up routes may remain as documented compatibility wrappers
during migration, but the canonical product model must have one message
identity with requested delivery, actual delivery, target/successor run, status,
timestamps, and reason. Old ProductStore rows and canonical events require an
explicit, tested mapping; do not maintain two durable queue authorities or
dual-fire two permanent event lifecycles.

#### F.2 Harness intervention boundary

"Immediate" means the earliest protocol-safe boundary, not cancellation in the
middle of a provider stream or tool side effect:

- finish the currently in-flight provider or tool call and durably record its
  observable outcome;
- satisfy already-committed provider tool-result pairing before another model
  request;
- drain accepted intervention messages before the next ordinary model turn,
  plan dispatch, or terminal Finalizer commit;
- append the visible assistant output and then the promoted user message to
  canonical history, rebuild the bounded prompt through the existing context
  authority, and continue the same run;
- let existing plan revision/reconciliation rules decide how remaining work
  changes instead of mutating the plan through a UI-only path.

A promoted message is not an approval decision, a `request_input` answer, a
capability grant, or implicit cancellation. If the run is blocked on one of
those typed interactions, the intervention waits until that protocol obligation
is explicitly resolved. Restart and reconnect recover the durable delivery
state; completed application is never replayed.

The new generic message lifecycle must be represented by canonical events for
queued, intervention-requested, applied-current-run, claimed-successor,
needs-attention, and revoked outcomes. Trace, TaskState/prompt checkpoint,
ProductStore projection, API SSE/replay, reports, Web reducers, and evidence
export must agree on identity and outcome without storing a second event log.

#### F.3 Chat-first information architecture

The default product job is: talk to the Agent, understand what it is doing, and
review the result. Information has three user-visible levels:

1. The main conversation shows user/assistant messages, one evolving
   human-readable activity summary per run, required approval/input, failures,
   and the final result.
2. A user-opened Run Inspector shows activity, plan progress, tools, changed
   files, tests, diffs, artifacts, and evidence for the selected run.
3. An explicit diagnostics view shows canonical trace, event sequence, raw
   run/job IDs, usage/context, prompt/cache identity, restore facts, and export.

The Inspector is closed by default and must not reserve a permanent third
column. Clicking an activity summary opens it on the corresponding historical
or active run; it follows the latest run only when the user has not selected an
older one. Closing it never hides an approval, required input, failure, or
unknown-effect state from the main conversation. Desktop uses an on-demand side
panel and narrow layouts use a focus-managed full-screen drawer or detail view.

Tool activity in the transcript is deterministically grouped from canonical
events and trusted tool metadata. It may say, for example, that files were read,
searched, changed, and tested; it must not invent an Agent thought process.
Unknown tools retain a neutral factual label. Every summarized item remains
reachable through Inspector details.

The composer contains the text input, send action, stop while running, compact
current-model entry, and one advanced-actions entry. Queued messages are shown
in conversation context with delivery state and eligible promote/revoke
actions; a persistent control queue and Steer/Follow-up mode selector are
removed from the default surface.

#### F.4 Interaction reliability and performance

Deliverables:

- fix streaming auto-scroll when message content grows while item count stays
  constant while preserving the user's reading position;
- add long-session pagination/windowing, bounded lazy rendering, stable prepend
  anchoring, and on-demand trace/tool-output loading;
- keep active and historical Run Inspector selection correct across refresh,
  SSE reconnect, exact-session navigation, and successor turns;
- retain safe rich-text URL handling, honest restore errors, accessibility
  roles, focus return/trapping, and fail-closed migration states;
- provide responsive provider setup, workspace selection, approval, input,
  cancellation, resume, artifact, diff, and evidence workflows in the default
  product shell;
- verify Desktop startup, shutdown, packaging, update/error surfaces, token
  handling, focus, keyboard, accessibility, and platform-specific paths.

Visual restyling is deliberately deferred to a later frontend-design skill
pass. This workstream does not choose colors, typography, radii, motion,
typewriter pacing, a Markdown replacement, a design-token rewrite, or an
animation system. Any later dependency or rendering change still requires
measured benefit, license/security review, bundle impact, and browser evidence.

#### F.5 Full-screen TUI parity

The existing bounded `rove tui` remains an optional terminal presentation over
the shared CLI Runtime, Engine, canonical events, state, and artifacts. This
program supersedes the earlier TUI non-goal that placed prompt queueing wholly
outside the bounded MVP, but only for the durable session-message lifecycle
defined in F.1 and F.2. It does not authorize a TUI-only queue or a second
product backend.

Required terminal-native behavior:

- keep one composer action: submitting while idle starts work and submitting
  during an active run durably queues the message in FIFO order; never require
  the user to select `Steer` or `Follow-up` before typing;
- project accepted messages immediately into the chronological timeline with a
  clear queued, intervention-requested, applied, successor-claimed,
  needs-attention, or revoked state;
- let the user select an eligible queued message and request promotion or
  revocation against the same durable message identity; retries, repeated key
  events, and redraws must not duplicate either action;
- call the shared product-message domain service through an in-process adapter
  so local deterministic TUI use does not require an API server. The canonical
  API is a peer adapter over the same contract. `TuiState`, reducers, effects,
  and channels may hold bounded presentation state, but cannot become a durable
  queue authority;
- keep approval and `request_input` modals authoritative over composer input.
  A queued message or promotion is never interpreted as an approval, input
  answer, capability grant, or cancellation, and waits when a typed protocol
  obligation blocks intervention;
- keep the main terminal surface conversation-first. Reuse bounded overlays or
  detail views for run activity, plan/tool details, changed files, tests,
  diffs, artifact metadata, evidence, and diagnostics instead of reserving a
  permanent Inspector pane. Raw run/job IDs, event sequence, prompt/cache
  identity, and restore facts belong in diagnostics, not the main timeline;
- bound long-session materialization and tool/output loading, preserve a stable
  scroll anchor when older rows are loaded, follow growing streamed content
  only while the user remains at the latest position, and never let updates or
  resize hide the composer or an actionable modal;
- restore queued-message state and exact delivery outcomes across session
  resume and process restart, and show successor-turn transitions without
  replaying a completed application;
- retain terminal capability gating, sanitization, secret redaction, alternate
  screen restoration, and honest unsupported/unverified platform states.

The TUI remains a single-foreground-session/run presentation. Mouse support,
inline image rendering, multiple active-session dashboards, background task
management, a TUI-specific provider/setup backend, and pixel-equivalent
Web/Desktop layout are outside this workstream.

Required TUI verification:

- reducer/effect and TestBackend coverage for idle submission, active-run
  queueing, ordered projection, promotion, revocation, needs-attention, and
  successor claim, including duplicate and stale actions;
- approval/input modal precedence and tests proving that paste, repeat/release,
  or mismatched message IDs cannot trigger promotion, revocation, approval, or
  input submission;
- bounded rendering at normal, narrow, and minimal terminal sizes, plus resize,
  streaming-follow, manual-scroll, history-prepend, and long-session cases;
- cancellation, EOF, draw failure, panic/unwind, resume, and terminal restore
  keep durable message state honest and release all process-local responders;
- Unix PTY smoke remains opt-in platform evidence; Windows ConPTY automation is
  reported as unverified until a real native gate exists and passes.

### G. Real-world evaluation and release confidence

Deliverables:

- deterministic fake-provider regression suite for every new behavior;
- OnCall/reference-agent suite remains independent and safety-gated;
- a small real-provider matrix for at least one native tool-use provider and one
  local/provider-compatible path when available;
- repository dogfood scenarios covering read-only analysis, bounded edits,
  tests, failed validation, cancellation, resume, stale observations, large
  outputs, MCP results, and approval boundaries;
- browser acceptance against a live local API, not only mocked APIs;
- live-browser and API race scenarios for active-run default queueing,
  queue-order preservation, promotion during provider generation and tool
  execution, promotion-versus-terminal completion, repeated idempotent clicks,
  revoke-versus-claim, refresh/restart recovery, and failure/cancelled
  confirmation;
- deterministic TUI reducer/render coverage for the same message lifecycle and
  terminal-native interaction rules, with platform PTY results classified
  separately from TestBackend evidence;
- clean-install Web, Desktop, and CLI/TUI smoke, including
  unconfigured/fake-provider onboarding;
- explicit residual-risk report for unsandboxed local shell execution, skipped
  external services, signing, platform coverage, and provider-specific limits.

## 5. Hard Contract Rules

- All interfaces continue to use the shared Engine/kernel and canonical events.
- Tool descriptions, prompts, procedures, MCP annotations, URLs, filenames, and
  model output never grant permission.
- Local paths remain bounded by the resolved workspace and validated again at
  execution boundaries.
- Completed mutations and completed plan work are never replayed on resume.
- A user message is durable before acknowledgement. Queueing, promotion,
  application, successor claim, recovery, and revocation preserve one identity
  and are idempotent under retry.
- Web, Desktop, and TUI may keep bounded local projection state, but none may
  maintain a private durable message queue or infer a different delivery state
  from presentation timing.
- Immediate intervention never interrupts an in-flight provider call or tool
  side effect, never grants approval or capability, and never bypasses Runtime
  budgets or terminal reconciliation.
- `trace.jsonl` records canonical facts, `task_state.json` records resumable
  state, and `report.json` remains a derived projection.
- Artifact content is bounded, redacted, typed, and served only through the
  canonical artifact authority.
- Fake-provider evidence proves deterministic behavior only; it does not prove
  external interoperability.
- Every current-state claim must point to code, tests, generated schema, or a
  reproducible gate.

## 6. Acceptance Package

The final package must contain:

- final commit and clean-tree record;
- Rust, Web, and focused CLI/TUI gate logs with real exit codes;
- live API/browser acceptance artifacts;
- deterministic benchmark and OnCall evidence with provenance;
- provider/MCP/Desktop/PTY gate results classified as passed, failed, skipped,
  or unverified;
- representative redacted trace/state/report/artifact projections;
- security review covering path, secret, approval, URL, MIME, output, timeout,
  cancellation, retry, resume, and unknown-effect behavior;
- updated `docs/runtime/acceptance-matrix.md` and `release-readiness.md`.

The program is not complete because a document says it is complete. Completion
requires the implementation, named evidence, current documentation, and a
reviewable clean branch to agree.

## 7. Audit And Correction Sources

The files below are retained so an implementer can reproduce a finding or
understand why this program selected or rejected an approach. They have no
independent deliverables, priorities, sequencing, acceptance criteria, or
completion status. Do not use them to expand this program. If a useful proposal
appears only in one of these files and not above, it is outside this round.

- [`2026-08-09-prompt-and-agent-intelligence.md`](2026-08-09-prompt-and-agent-intelligence.md)
  — 2026-08-09 prompt/tool-call current-state audit and correction evidence;
- [`2026-08-09-codebase-understanding.md`](2026-08-09-codebase-understanding.md)
  — 2026-08-09 retrieval/search current-state audit and reference comparison;
- [`2026-08-09-tool-output-references.md`](2026-08-09-tool-output-references.md)
  — 2026-08-09 context-cost evidence and artifact-authority correction;
- [`2026-08-09-frontend-elegance-reference.md`](../design/2026-08-09-frontend-elegance-reference.md)
  — current Web audit and third-party presentation reference only;
- [`2026-08-09-deferred-capabilities.md`](2026-08-09-deferred-capabilities.md)
  — intentionally deferred Subagent/sandbox boundary record and historical G-F rationale;
- [`2026-07-16-grok-build-reference-and-tui-design.md`](../design/2026-07-16-grok-build-reference-and-tui-design.md)
  — implemented bounded TUI baseline and terminal-safety reference; its earlier
  prompt-queue non-goal is superseded only by F.5 above;
- [`2026-08-07-post-coding-tool-v2-full-delivery.md`](2026-08-07-post-coding-tool-v2-full-delivery.md)
  — verified completed foundation and historical delivery record, not remaining
  implementation scope.
