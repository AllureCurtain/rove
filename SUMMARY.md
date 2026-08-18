# Review workflow implementation summary

The `feature/read-only-review` worktree implements an end-to-end hard read-only
code Review flow over the existing Runtime Engine.

## Delivered

- Runtime Git target capture for uncommitted, base, and commit targets, with
  immutable snapshot reads, state hashes, bounded diffs, digest/stale checks,
  and external Review state.
- A Review-only registry, read-only execution environment, exact pre-dispatch
  descriptor allowlist, no hooks/MCP/shell/memory writes, and approval that
  cannot grant mutation authority.
- Versioned structured findings with bounds, path/location validation,
  deduplication, secret redaction, unchecked ranges, and conservative result
  finalization.
- Review-safe event, task-state, report, SSE, terminal, and Tool Artifact
  persistence. Captured source bytes remain only in the external target
  snapshot, not normal run artifacts.
- ProductStore schema v14 with Review rows/findings, migration from v13,
  idempotency, active-target single flight, pagination, cancellation, stale
  projection, and conservative process-restart recovery.
- Product API/OpenAPI lifecycle routes, CLI text/JSON/JSONL output and exit
  codes, and Web composer/Inspector launch, status, pagination, cancellation,
  and file navigation.
- Runtime/design/release documentation and deterministic Rust/Web evidence.

## Compatibility

Normal runs keep `RunMode::Normal`; their registry, artifacts, event payloads,
approval, hooks, and execution environment retain existing behavior. The v14
database migration is additive. The v13 reconciliation of the two parallel v12
productization layouts remains intact and is tested before v14 is applied.

## Not part of this change

This worktree does not implement user-state-directory migration, managed
worktrees, background task center, full-history pagination, vector RAG, LSP,
multi-agent supervision, automatic fixes, or a private TUI Review backend. It
does not modify `.worktrees/user-state-migration`.

External Provider, hosted/official MCP, ConPTY, macOS/Linux packaging,
signing, installed-Desktop, and broader stress/soak evidence remain unverified.
