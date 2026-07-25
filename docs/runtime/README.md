# Runtime Documentation

This directory is the authoritative current-state documentation surface. It
summarizes the implemented runtime, API, Web M1 product shell, and remaining
gaps. Future Web Complete and Desktop contracts remain under `docs/design/` and
`docs/plans/` until their code and tests land.

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

The Web product line is tracked separately:

- Web M1 is implemented; its ledger is
  [`docs/plans/2026-07-25-web-management-m1.md`](../plans/2026-07-25-web-management-m1.md).
- M1 product-shell browser coverage is mock-backed. The gated `local-full`
  real-API suite targets advanced `/dev/workbench`; live-API acceptance of `/`
  remains Web Complete C3 work.
- Web Complete C0–C3 is active but not implemented; follow
  [`docs/design/2026-07-26-web-complete-design.md`](../design/2026-07-26-web-complete-design.md)
  and the
  [`Web → Desktop coordinator plan`](../plans/2026-07-25-web-desktop-master-delivery.md).
- No Tauri Desktop host exists. `desktop-workspace-spec.md` is an automation
  workspace note, not the Desktop product-shell design.

Historical May/June hardening and RAG design notes live under
[`docs/Archive/design/`](../Archive/design/). These runtime docs describe what
exists now and where the remaining gaps are.
