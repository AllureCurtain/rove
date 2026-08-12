# P0 - Codebase Understanding

> Status: **Current-state correction and audit evidence / Not an implementation plan.**
> The 2026-08-10 program selectively incorporates findings into Workstream C:
> [`2026-08-10-post-full-delivery-productization.md`](2026-08-10-post-full-delivery-productization.md).
> Sections named `Proposed Design` or `Acceptance Criteria` are retained analysis
> and do not create future work. Only requirements restated in the 2026-08-10
> program are authorized for implementation.
>
> Scope constraint from the review: **stay lightweight.** The outcome must not
> introduce a semantic index, a vector database, or a language server.

## 0. Headline Defect

**`.gitignore` is not consulted anywhere in the search path. This is the single
most damaging defect in the current implementation, and the highest-value fix in
this document.**

It is worth stating before anything else because it is cheap to fix, it is not a
capability gap, and it silently defeats the bounds that are supposed to make
search safe. In a repository containing `target/`, `node_modules/`, or `.next/`,
traversal scans build output until `max_files_scanned` trips - so the bound fires
on generated noise and the real matches are never reached. The tool reports a
successful, bounded search that found nothing. There is no signal to the model
that the result is meaningless.

Every other item in this document is a capability the agent does not yet have.
This one is an existing capability producing confidently wrong answers, which is
strictly worse. Fix order follows accordingly: section 4.1 first, independently of
the rest.

## 1. Current State

`runtime/src/tools/search.rs` is `regex` plus `walkdir`. Verified dependency
check on 2026-08-09: the workspace has **no** `tree-sitter`, no `tantivy`, no
LSP crate, no embedding or vector store dependency.

What exists today:

| Tool | Mechanism | Bounds |
|---|---|---|
| `search_code` | `RegexBuilder` over `walkdir` traversal | `max_matches`, `max_files_scanned`, `max_output_bytes`, timeout |
| `glob_paths` | Hand-rolled glob compiled to regex (`compile_glob`, `search.rs:450`) | Bounded |
| `list_directory` | Direct traversal | `recursive`, `page_limit` |
| `read_file` | Bounded ranged read | Byte bounds, observation recorded |

The glob implementation translates `*` to `.*` and `?` to `.`. It does not
support `**`, brace expansion, or character classes. `.gitignore` is **not**
consulted anywhere in the audited baseline. The final full-delivery branch must
be rechecked before implementation because a later tool wave may have changed
traversal or result behavior without closing this plan.

### 1.1 Post-full-delivery boundary

This plan improves bounded workspace retrieval; it does not replace the Agent
profile, procedure catalog, or canonical Tool Artifact authority. Search results
must remain model-readable within a bounded projection, with larger payloads
represented by the existing artifact/reference contracts. Ignored-file support
must never become an implicit permission to expose secrets or scan outside the
workspace.

## 2. Concrete Consequences

1. **No `.gitignore` awareness.** See section 0 - this is the headline defect and
   is called out there rather than ranked among the capability gaps below.
2. **Regex cannot answer structural questions.** "Every caller of X" misses
   re-exports, trait implementations, and macro-generated call sites.
3. **No repository map.** The model accumulates structural knowledge through
   repeated `list_directory` calls, consuming turns from the same budget that the
   evidence run already exhausted.
4. **No context lines.** A match returns its own line. The model must issue a
   second `read_file` to see why the match matters, spending another turn.

Point 4 compounds directly with the G-A finding: turns wasted on navigation are
turns unavailable for the task.

## 3. Reference Designs

### PI - shell out to `ripgrep` and `fd`

`pi/packages/coding-agent/src/core/tools/grep.ts`:

- Resolves `rg` through `ensureTool("rg", true)`, which **downloads the binary**
  if absent, then `spawn`s it.
- Supports a `context` parameter; when set, reads the file and slices
  `[line - context, line + context]` (`grep.ts:260`).
- Streams matches during execution, formats after `rg` exits.
- Maps a missing binary and a non-zero exit to distinct errors.

`find.ts` does the same with `fd`, and its description advertises `.gitignore`
compliance and the exact truncation limits.

Notable: PI defines a `customOps` seam so the default `rg`/`fd` path can be
replaced (used by the sandbox extension). Structurally that is the same idea as
rove's `ExecutionEnvironment` ports.

**Assessment for rove:** the context-lines behavior and the `.gitignore`
compliance are directly worth adopting. **Auto-downloading a binary is not** -
it is a supply-chain and trust surface that contradicts our fail-closed posture.

### Claude Code - separate Glob and Grep, plus delegation

From `learn-claude-code/s02_tool_use`: `Glob` and `Grep` are distinct tools, both
marked read-only and concurrency-safe. The `isConcurrencySafe` flag equals
`isReadOnly` for Bash. rove already has this property expressed as
`parallel_safe` on `ToolDescriptor`, so parallel search is already available to
us.

The larger lever is delegation: broad exploratory search is handed to a subagent
whose intermediate output does not enter the main context. That is gap G-D and is
deferred, so **rove must get more value per search call** instead.

### Hermes - `rg --files` for enumeration

`hermes-agent/agent/context_references.py:541` invokes `["rg", "--files", ...]`
to enumerate tracked files, relying on ripgrep's `.gitignore` handling rather
than reimplementing traversal. Narrow use, but it confirms the same conclusion:
ignore-awareness is worth borrowing from ripgrep semantics.

### Aider - tree-sitter repository map

Not in the local set, but the relevant reference point: parse with tree-sitter,
extract top-level definitions, rank by graph centrality, render a compact map
into the prompt. Highest structural value, and the heaviest option. Out of scope
under the lightweight constraint, but recorded as the natural follow-on.

## 4. Proposed Design

Four changes, ordered by value per unit of effort. Each is independently
shippable.

### 4.1 Ignore-awareness (highest value, lowest cost)

Adopt `.gitignore` and `.ignore` semantics in `search_code`, `glob_paths`, and
recursive `list_directory`.

Implementation: replace the `walkdir` traversal with the `ignore` crate, which is
the library ripgrep itself uses. It is a pure-Rust dependency, no external
binary, and it brings correct precedence handling for nested ignore files.

Requirements:
- An explicit `include_ignored: bool` argument, defaulting to `false`.
- Ignore semantics apply to traversal only. They **never** widen the workspace
  path boundary.
- Hidden files remain excluded by default, as today.

### 4.2 Context lines in `search_code`

Add an optional `context: u32` argument, bounded (proposal: max 10). Return the
surrounding lines with the match, following PI's slicing approach.

This directly reduces round trips: today a match costs one search turn plus one
read turn.

Byte accounting must include context lines so `max_output_bytes` stays honest.

### 4.3 Real glob semantics

Replace the hand-rolled `compile_glob` with a maintained glob implementation
supporting `**`, brace expansion, and character classes. Reject patterns that
attempt traversal outside the workspace before compilation, preserving current
boundary behavior.

### 4.4 Bounded repository map (largest change, still lightweight)

A deterministic, non-parsing structural summary emitted as a dynamic prompt
section (depends on G-A's section model):

```text
Repository map (bounded):
  Cargo workspace, 8 members
  models/     14k lines   protocol, provider adapters, routing
  core/        2k lines   embeddable agent, tool contracts
  runtime/    27k lines   engine, state, tools, memory, planning
  apps/api/   28k lines   HTTP/SSE, product control plane
  ...
```

Derivation rules:
- Source of truth is manifest files (`Cargo.toml` members, `package.json`
  workspaces), not heuristics.
- Directory descriptions come from existing documentation when present, otherwise
  omitted. **Never** invented.
- Hard byte ceiling; truncation is explicit and marked.
- Computed on demand or from a content-addressed workspace/manifest fingerprint.
  It may participate in capability metadata, but it must not be regenerated and
  injected into every model turn when the inputs did not change.

Explicitly **not** in scope: symbol extraction, call graphs, centrality ranking.
That is the tree-sitter follow-on.

## 5. Risks

| Risk | Mitigation |
|---|---|
| `ignore` crate changes result sets and breaks existing tests | Land behind an explicit argument default; update affected tests deliberately, not by loosening assertions |
| Context lines inflate output past bounds | Count context bytes against the same `max_output_bytes`; add a test at the boundary |
| Ignore semantics used to escape the workspace | Boundary enforcement stays in `LocalFileSystem`; add a test proving an ignore file cannot widen scope |
| Repository map drifts from reality | Derive from manifests and verified docs; bind cache identity to manifest/workspace fingerprints and invalidate explicitly |
| Scope creep into a semantic index | The lightweight constraint is recorded here as a requirement, not a preference |

## 6. Rejected Options

| Option | Why rejected |
|---|---|
| Auto-download `rg`/`fd` like PI | Supply-chain and trust surface; contradicts fail-closed posture. Use the `ignore` crate in-process instead |
| Shell out to system `rg` when present | Non-deterministic across machines; breaks the deterministic benchmark guarantee |
| Vector or embedding index | Explicitly out of scope per the product boundary in `docs/runtime/` |
| LSP integration | Requires per-language servers and lifecycle management. Disproportionate |
| tree-sitter symbol map now | Highest value long-term, but exceeds the lightweight constraint for this pass. Record as follow-on |

## 7. Acceptance Criteria

1. `search_code` in this repository with `target/`, `node_modules/`, and
   `.next/` present returns real source matches without exhausting its scan
   bound on generated noise.
2. Traversal order, match order, and truncation metadata are deterministic across
   repeated runs on the same workspace.
3. `context` returns requested surrounding lines, coalesces overlapping ranges,
   and counts every returned byte against `max_output_bytes`.
4. `glob_paths` handles `**/*.rs`, `{a,b}`, and `[0-9]`, while rejecting path
   escapes and testing symlink/reparse boundaries.
5. An `.gitignore` or `.ignore` file cannot cause any tool to read outside the
   workspace root, and ignored-file opt-in cannot bypass sensitive-path policy.
6. Binary, hidden, ignored, missing, and oversized files produce explicit
   bounded outcomes rather than silently looking like empty search results.
7. The repository map renders under its byte ceiling, truncates explicitly, and
   contains no invented description; unchanged inputs reuse the same digest.
8. `agent-smoke` evidence is recorded before and after in a separate artifact
   directory, and `docs/runtime/subsystems.md` is updated only after code/tests
   land.
