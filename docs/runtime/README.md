# Runtime Documentation

This directory is the authoritative runtime documentation surface. It summarizes the implemented architecture and the remaining gaps against the runtime hardening target.

## Documents

| File | Purpose |
|---|---|
| [mvp-definition.md](mvp-definition.md) | Current local-first MVP boundary, included capabilities, exclusions, golden paths, and verification baseline. |
| [architecture.md](architecture.md) | Top-level runtime architecture and cross-module boundaries. |
| [react-loop.md](react-loop.md) | Plan outside, ReAct inside runtime loop explanation and pico relationship. |
| [subsystems.md](subsystems.md) | Config, state/job, context, provider, memory, tool, API/security, RAG, web, and CI subsystem notes. |
| [implementation-status.md](implementation-status.md) | Current implementation vs target architecture matrix. |
| [implementation-guide.md](implementation-guide.md) | Maintainer-focused implementation guide with startup paths, runtime flow, state artifacts, verification, and known gaps. |
| [acceptance-matrix.md](acceptance-matrix.md) | M0-M6 acceptance criteria mapped to concrete verification commands. |
| [integration-testing.md](integration-testing.md) | End-to-end integration profiles, required local-full baseline, optional provider/MCP/RAG gates, and runner design. |
| [full-integration-runbook.md](full-integration-runbook.md) | New-session runbook for full API/Web/provider/MCP/stress integration testing across official APIs, relay/gateway APIs, and local providers. |
| [provider-smoke.md](provider-smoke.md) | Opt-in real-provider verification for OpenAI-compatible, Anthropic, and Ollama paths. |
| [release-readiness.md](release-readiness.md) | MVP release checklist covering verification, provider smoke, packaging, security posture, and out-of-scope reminders. |
| [browser-workspace-spec.md](browser-workspace-spec.md) | Future Browser workspace design note; not a current runtime implementation. |
| [desktop-workspace-spec.md](desktop-workspace-spec.md) | Future Desktop workspace design note; not a current runtime implementation. |

## Source Design

The current architecture is based on:

- `docs/superpowers/specs/2026-05-24-rove-runtime-hardening-design.md`
- `docs/superpowers/specs/2026-05-24-rag-pipeline-hardening-design.md`

Those specs explain the target direction. These runtime docs describe what exists now and where the remaining gaps are. Older top-level design docs are historical references unless they explicitly point back here.
