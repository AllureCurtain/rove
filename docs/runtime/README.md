# Runtime Documentation

This directory is the authoritative current-state documentation surface. It
summarizes the implemented runtime, API, Web M1 product shell, verified Web
Complete C0-C3 implementation, and CDH G1-G7 control/evidence/Settings
completion. C1-C3 are integrated on `main` through PRs #24-#26; CDH G1-G7
merged through PR #29 at `f9e88a7`. Post-Coding-Tool full delivery merged
through PR #30 at `4b740d3`, followed by the whitespace-only PR #31 cleanup at
`1b57b36`. The deterministic live-API gates passed. The current source includes
the full-delivery shared
kernel/lifecycle, MCP Streamable HTTP plus rich Tool Result/Artifact and live
catalog refresh, AgentDefinition/instruction/procedure checkpoints, and the
implemented Tauri Desktop D0 host. Desktop reuses the API router and ProductStore;
it is not a second runtime or state authority. Current evidence is Windows-only
for packaging and process launch; macOS/Linux packaging remains unverified.
The current source also implements the 2026-08-12 user-owned Provider catalog
and TUI model-selection slice: `~/.rove/config.toml`, authority-aware loading,
catalog CAS/atomic writes, API/Web catalog convergence, per-turn CLI assembly,
secret-free run model snapshots, legacy migration, and TUI `/model`. Real
external-provider interoperability for this slice has not been run.

New maintainers should start with [`docs/ONBOARDING.md`](../ONBOARDING.md), then use this directory for current subsystem truth.

## Documents

| File | Purpose |
|---|---|
| [mvp-definition.md](mvp-definition.md) | Current local-first MVP boundary, included capabilities, exclusions, golden paths, and verification baseline. |
| [architecture.md](architecture.md) | Top-level runtime architecture and cross-module boundaries. |
| [react-loop.md](react-loop.md) | Plan outside, ReAct inside runtime loop explanation and pico relationship. |
| [subsystems.md](subsystems.md) | Config, state/job, context, provider, memory, tool, API/security, workspace retrieval, web, and CI subsystem notes. |
| [implementation-status.md](implementation-status.md) | Current implementation vs target architecture matrix. |
| [implementation-guide.md](implementation-guide.md) | Maintainer-focused implementation guide with startup paths, the bounded `rove tui` navigation/timeline/interaction contract, terminal verification boundaries, runtime flow, state artifacts, and known gaps. |
| [acceptance-matrix.md](acceptance-matrix.md) | M0-M6 acceptance criteria mapped to concrete verification commands. |
| [integration-testing.md](integration-testing.md) | End-to-end integration profiles, required local-full baseline, optional provider/MCP gates, and runner design. |
| [full-integration-runbook.md](full-integration-runbook.md) | New-session runbook for full API/Web/provider/MCP/stress integration testing across official APIs, relay/gateway APIs, and local providers. |
| [provider-smoke.md](provider-smoke.md) | Opt-in real-provider verification for OpenAI, Anthropic, and Ollama paths. |
| [release-readiness.md](release-readiness.md) | MVP release checklist covering verification, provider smoke, packaging, security posture, and out-of-scope reminders. |
| [browser-workspace-spec.md](browser-workspace-spec.md) | Future Browser workspace design note; not a current runtime implementation. |
| [desktop-workspace-spec.md](desktop-workspace-spec.md) | Future Desktop workspace design note; not a current runtime implementation. |

## Source Design

The current architecture is based on:

- [`docs/design/2026-07-22-modular-workspace-architecture.md`](../design/2026-07-22-modular-workspace-architecture.md)
- [`docs/design/2026-07-23-provider-layer-redesign-design.md`](../design/2026-07-23-provider-layer-redesign-design.md)
- [`docs/design/2026-07-24-cleanup-and-naming-decisions.md`](../design/2026-07-24-cleanup-and-naming-decisions.md)

Partially implemented evolution records are:

- [`docs/design/2026-07-14-agent-execution-lifecycle-design.md`](../design/2026-07-14-agent-execution-lifecycle-design.md)
- [`docs/design/2026-07-14-agent-definition-and-procedural-knowledge-design.md`](../design/2026-07-14-agent-definition-and-procedural-knowledge-design.md)
- [`docs/design/2026-07-15-mcp-streamable-http-and-tool-artifacts-design.md`](../design/2026-07-15-mcp-streamable-http-and-tool-artifacts-design.md)

The Web product line is tracked separately:

- Web M1 is implemented; its ledger is
  [`docs/plans/2026-07-25-web-management-m1.md`](../plans/2026-07-25-web-management-m1.md).
- Product-shell browser evidence has two explicit layers. `shell.spec.ts`,
  `continuity.spec.ts`, `settings.spec.ts`, `migration.spec.ts`, and
  `polish.spec.ts` use browser-boundary mocks for broad deterministic state,
  race, recovery, and visual checks. The gated `local-full` suite runs
  `real-api.spec.ts` against a live Rust API; its C3 run passed all three cases:
  migration before catalog boot, exact A/B session continuity plus refresh and
  product interactions, and a bounded advanced `/dev/workbench` smoke.
- Web Complete C0 is implemented: the API owns `product.sqlite`, product
  workspace/session/profile/preferences CRUD, exact product-session/runtime
  bindings, a canonical-event transcript read projection, and
  strict/idempotent M1 migration. Migration preparation is deadline-bounded;
  the supervised apply phase survives an HTTP disconnect, uses a durable
  preflight baseline and preference revision CAS, and reserves canonical
  workspace runtime databases before committing verified bindings. Typed Web
  client and migration modules are present.
- Web Complete C1 is implemented: the default `ProductApp` consumes the C0
  workspace/session/preferences/profile/transcript client, restores canonical
  history with explicit partial/error/retry states, uses durable workspace,
  session, and Settings routes, sends exact `product_session_id` turns, and
  reattaches only the focused live job while polling durable background status.
  Ambiguous job-start responses use bounded binding reconciliation and never
  trigger an automatic duplicate submission. Provider profiles and selection
  are API-authoritative.
- Web Complete C2 is implemented: preferences use revision CAS and a durable
  default approval policy, product jobs honor that default, bounded Memory and
  runtime-health APIs back the UI, provider profiles support complete CRUD,
  and all nine Settings sections expose tested catalog, session, memory,
  runtime, approval, keyboard, or developer capabilities.
- Web Complete C3 is implemented and verified on `main`.
  `M1MigrationGate` runs before API-authoritative catalog boot, permits the shell
  only after `not_needed` or verified `complete`, preserves exact retry payloads,
  and keeps invalid or uncertain imports fail closed. C3 also completes the
  responsive, focus, reduced-motion, and state polish and moves deterministic
  live-API acceptance to the default `/` product shell while retaining one
  bounded `/dev/workbench` check. The provider runner now targets an exact
  product session and correlates the browser's returned job/run IDs, but no
  external-provider C3 gate has been run. Follow
  [`docs/design/2026-07-26-web-complete-design.md`](../design/2026-07-26-web-complete-design.md)
  for the completed design record.
- CDH G1-G7 are implemented and verified on `main` through PR #29. The product
  has durable Steer/Follow-up controls, terminal-boundary Fork/lineage, immutable
  session run configuration snapshots, usage/context/cost, bounded files and
  artifacts, image validation, run/Git diff, redacted evidence export, and a
  workspace-scoped MCP catalog shared by Settings and job assembly. See the
  [`acceptance matrix`](acceptance-matrix.md) and the
  [`completed CDH plan`](../plans/2026-08-03-cdh-alder-merge.md).
- The completed
  [`Kernel, Message, and Provider implementation record`](../plans/2026-08-06-kernel-message-provider-implementation.md)
  covers typed messages, provider normalization, and shared-kernel migration.
- The completed
  [`Tool Schema and Runtime validation record`](../plans/2026-08-07-authoritative-tool-schema-runtime-validation.md)
  covers bounded schema validation, atomic catalogs, model preflight, and
  Runtime capability snapshots.
- The completed
  [`Project Trust, Execution Environment, and Coding Tools implementation
  record`](../plans/2026-08-06-project-trust-execution-tools-implementation.md)
  covers durable trust, Runtime-owned execution adapters, and the Coding Tool
  foundation.
- Coding Tool V2 is implemented and verified by the
  [`Coding Tool V2 implementation plan`](../plans/2026-08-07-coding-tool-v2-implementation.md).
- A Tauri Desktop product host exists in `apps/desktop` on `main` through PR
  #30 and reuses the API router, ProductStore, and shared static Web build.
  `desktop-workspace-spec.md` remains an automation workspace note, not the
  Desktop product-shell design.
- The
  [`User Provider Configuration and TUI Model Selection design`](../design/2026-08-12-user-provider-config-and-tui-model-selection-design.md)
  is implemented through Phase 0-5. Its configured-model catalog, CAS,
  migration, CLI/TUI, API/Web, and resume contracts have deterministic test
  coverage. The optional real external-provider smoke remains unverified, so
  this is not an interoperability claim.
- The next productization work is proposed, not implemented, in
  [`2026-08-10-post-full-delivery-productization.md`](../plans/2026-08-10-post-full-delivery-productization.md).
  Its dated audit inputs are evidence and rationale, not independent plans.

Historical May/June hardening and RAG design notes live under
[`docs/Archive/design/`](../Archive/design/). These runtime docs describe what
exists now and where the remaining gaps are.
