# Runtime Documentation

This directory is the authoritative runtime documentation surface. It summarizes the implemented architecture and the remaining gaps against the runtime hardening target.

## Documents

| File | Purpose |
|---|---|
| [architecture.md](architecture.md) | Top-level runtime architecture and cross-module boundaries. |
| [subsystems.md](subsystems.md) | Config, state/job, context, provider, memory, tool, API/security, RAG, web, and CI subsystem notes. |
| [implementation-status.md](implementation-status.md) | Current implementation vs target architecture matrix. |

## Source Design

The current architecture is based on:

- `docs/superpowers/specs/2026-05-24-rove-runtime-hardening-design.md`
- `docs/superpowers/specs/2026-05-24-rag-pipeline-hardening-design.md`

Those specs explain the target direction. These runtime docs describe what exists now and where the remaining gaps are. Older top-level design docs are historical references unless they explicitly point back here.
