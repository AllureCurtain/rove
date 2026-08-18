# Hard read-only Review workflow

## Contract

Review analyzes a captured Git change target without granting the model a
mutation path. It is a restricted profile over the shared Runtime Engine and
canonical event lifecycle; it is not a second agent loop, queue, report
authority, or permission system.

Supported targets are:

- `uncommitted`: `HEAD` versus the current index/worktree, including untracked
  files;
- `base <revision>`: the resolved base commit versus the current worktree;
- `commit <revision>`: the resolved commit's parent versus that commit.

The launch path captures an immutable `ReviewTargetSnapshot` with separate
HEAD/index/worktree states, bounded diffs, content hashes, rename/binary facts,
the resolved revision, and a stable digest. Every model-visible Review read
tool closes over that snapshot. A fresh capture before finalization and on
result access detects target drift.

## Read-only proof

The restriction is enforced independently at three boundaries:

1. Bootstrap builds a Review-only `ToolRegistry`. It contains snapshot-backed
   read/list/glob/search/repository-map/diff tools, bounded artifact resolution,
   and the one-shot finding submitter. File mutations, shell, checkpoints,
   memory writes, request-input, MCP, and workspace hooks are absent.
2. `LocalExecutionEnvironment::read_only` denies filesystem writes and every
   process, observation, background, PTY, and checkpoint capability. The local
   filesystem and process adapters also fail closed if called directly.
3. `Executor` binds the exact pinned tool name and capability to a compile-time
   Review allowlist before hooks or approval. Approval is fixed to `Never` and
   cannot widen this list.

Review state is resolved outside the target workspace. The target snapshot is
the only durable location allowed to contain captured source bytes. Read/search
outputs remain available to the in-process model but are not retained as Tool
Artifacts. Review canonical events, task state, report, SSE, and terminal output
use a shared redacted persistence projection.

## Finding and result contract

`ReviewResult.schema_version` is `1`. The Runtime owns the result used by CLI,
API, ProductStore, and Web. A finding contains a stable ID, severity,
confidence, category, workspace-relative path, validated location, bounded
title/explanation/evidence, rule, suggestion, and status.

The model submits findings once through `review_submit_findings`. Before a
finding becomes durable, Runtime code:

- enforces JSON-schema counts and text bounds;
- normalizes and bounds the path to the captured target;
- validates line/column locations against captured text when possible;
- deduplicates stable finding identities;
- redacts secret-shaped assignments and sensitive snippets;
- records invalid, unvalidated, truncated, and unchecked portions;
- derives the conclusion from runtime facts rather than model prose.

Conclusions are `pass`, `findings`, `partial`, `stale`, `unavailable`,
`cancelled`, and `error`. Warnings or incomplete runtime artifacts make a
nominal no-finding result conservative (`partial`); no findings never erase an
unchecked range.

## Product surfaces

CLI:

```powershell
rove review
rove review --base main --format json
rove review --commit HEAD --format jsonl
```

Exit code `0` means a completed pass or a completed Review with findings;
`2` means partial, stale, unavailable, or cancelled; and `3` means an
internal Review error. Ctrl-C is surfaced as cancellation, with conventional
exit code `130` when the CLI owns the signal.

Product API:

- `POST /product/sessions/{session_id}/reviews`
- `GET /product/sessions/{session_id}/reviews`
- `GET /product/reviews/{review_id}`
- `GET /product/reviews/{review_id}/findings`
- `POST /product/reviews/{review_id}/cancel`

ProductStore schema v14 persists the bounded Review projection and finding
pages. Idempotency binds one key to one target digest, and one session/digest
may have only one active Review. On API restart, stranded `queued` or `running`
rows become `needs_attention`; they are not silently replayed.

Web exposes a Review launcher in the chat composer and a Review Inspector tab.
It renders loading, empty, running, pass, findings, partial, stale/
needs-attention, unavailable, cancelled, and error states, supports cancellation
and finding pagination, and opens a finding path/line in Files.

## Current limits

- Review does not apply fixes, patches, checkout, commit, or rollback.
- The initial snapshot bounds entries, diff bytes, and materialized text; every
  omitted portion remains visible in result facts.
- Interrupted API Review jobs are conservatively classified, not resumed.
- External Provider interoperability, real third-party MCP, Review-specific
  browser E2E, ConPTY, packaging/signing, and installed-Desktop gates are not
  claimed by deterministic local evidence.

See `docs/runtime/subsystems.md`,
`docs/design/2026-08-16-read-only-review-workflow-design.md`, and
`VERIFICATION.md` for the detailed implementation and evidence boundary.
