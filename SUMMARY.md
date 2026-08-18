# User State Directory Migration — Summary

> Status: **Implemented** in `feature/user-state-migration` from baseline
> `5fe9d70`.

## Outcome

Rove no longer defaults durable runtime data to each project's `.rove/`
directory. First-party CLI, TUI, API, embedded Desktop host, and product job
assembly now share one bootstrap-owned path contract:

```text
<data_root>/
  product.sqlite
  workspaces/<storage_key>/
    workspace.json
    state.sqlite
    runs/
    memory/
    mcp_servers.json
    session-model-selections/
    circuit_breakers.json
    tasks/
    repl_history
```

Windows defaults to `%LOCALAPPDATA%\rove`, macOS to
`~/Library/Application Support/rove`, and Linux to
`${XDG_DATA_HOME:-~/.local/share}/rove`. `ROVE_DATA_ROOT` and embedding
overrides must be absolute. Canonical workspace identity, Windows case/8.3/
extended-path handling, and `workspace.json` prevent silent cross-workspace
reuse. ProductStore remains one API-global authority at
`<data_root>/product.sqlite`.

Server-owned M1 import discovery keeps reading legacy `.rove` while the
contract directory contains only its identity marker, then switches to the
contract once `state.sqlite` or `runs/` materializes. Contract-state artifacts
outside the project root are accepted only after the matching marker is
verified.

Project-owned `.rove/config.toml`, `.env`, `AGENTS.md`, and AgentDefinition
sources remain in the project and remain Project Trust gated. The default MCP
catalog moves to user state; an explicitly configured project catalog keeps
its old semantics. Migration never grants `mcp_processes` or any other Trust
capability.

Before migration, MCP reads retain legacy compatibility without materializing
an empty user-state directory. The first Product Settings mutation validates
the request, creates/verifies the workspace marker, promotes the legacy catalog
once under the destination lock, and then mutates the contract catalog. Once
that catalog exists it is authoritative, so later legacy edits or deletions in
Settings cannot overwrite or resurrect old servers.

## Migration and resume

`rove state paths` reports the resolved contract and workspace identity.
`rove state migrate` is a zero-write dry-run by default; `--apply`, explicit
`backup-target` conflict handling, and `--prune-legacy` provide a conservative
copy-first migration.

The engine bounds depth, entries, and bytes; refuses source/target symlink
escapes; snapshots SQLite with `VACUUM INTO`; serializes workspace and global
ProductStore migration; records synced prepared/final journal entries; never
silently overwrites conflicts; and prunes only revalidated migrated files.
Corrupt sources, locks, invalid roots, and unknown files remain visible and
leave the legacy source readable.

Runtime SQLite indexes historically stored absolute artifact paths. Migration
now rebases only paths below the legacy state root, transactionally inside the
temporary snapshot before atomic publication. A real `StateStore` regression
proves the same `run_id` resumes after `--prune-legacy` and no indexed path
points back into `.rove`.

## Compatibility and scope

- Explicit state/memory/MCP paths retain historical resolution and external
  path validation.
- Direct runtime embedding defaults remain deterministic and do not discover a
  home directory; product entry points inject the resolved contract.
- Canonical events, Provider/ToolRegistry snapshots, approval policy, run IDs,
  and artifact bytes are not replayed or rewritten.
- `state repair` and `state cleanup` remain separate existing operations.
- No Web path resolver, second state authority, vector RAG, review workflow, or
  automatic Trust grant was introduced.

See [STATE_LAYOUT_AND_MIGRATION.md](STATE_LAYOUT_AND_MIGRATION.md) for the
operator contract, [VERIFICATION.md](VERIFICATION.md) for evidence, and
[DIFF_SUMMARY.md](DIFF_SUMMARY.md) for the file-level change map.
