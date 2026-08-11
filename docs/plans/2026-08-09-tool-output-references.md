# P1 - Tool Output References

Status: **Current-state correction and audit evidence / Not an implementation plan**
Related audit incorporation: Workstream D in
[`2026-08-10-post-full-delivery-productization.md`](2026-08-10-post-full-delivery-productization.md)

This document records a context-economy finding against the assumed completed
Rich Result and Tool Artifact contracts. Sections describing an approach or
acceptance criteria are retained analysis and do not create future work. Only
requirements restated in the 2026-08-10 program are authorized for
implementation.

## 1. Problem

Full tool output text is inserted verbatim into conversation history, and stays
there for every subsequent turn of the run. Reading one 4KB file costs 4KB on
that turn and on all following turns in the same step. Reading it twice costs it
twice.

This is a context-budget defect, not an audit concern. The audit requirement is
already satisfied elsewhere and must not change.

## 2. Evidence

From `.rove/runs/01KZGMB9M3DEB86R6CQCQ96TZW/` - task "Read Cargo.toml and
README.md, then explain in 3 short bullets", planned mode, terminated
`budget_exhausted` / `step_limit`.

Prompt tokens per model turn within the single step:

| Turn | `prompt_tokens` |
|---|---|
| 1 | 3942 |
| 2 | 4649 |
| 3 | 8000 |
| 4 | 8073 |

Cumulative `token_usage.prompt_tokens` for the step: 24664. Completion tokens on
turn 3: 48. The step consumed its entire four-turn budget and produced no answer.

`task_state.json` `history` entries 3 and 9 are byte-identical - the full text of
`Cargo.toml`, present twice. Entry 9 exists because the model reacted to a
validation error by abandoning `search_code` and re-reading a file it had already
read (see the error-message analysis in
`2026-08-09-prompt-and-agent-intelligence.md` section 4.5).

Two failures compound here, and they are separable:

- The model should not have re-read the file. That is a prompt and
  error-message problem, tracked in the prompt document.
- Re-reading it should not have cost a second full copy of the file. That is this
  document.

Fixing only the prompt leaves the underlying cost in place for every legitimate
repeated read.

## 3. What Must Not Change

- `trace.jsonl` continues to record canonical event facts, bounded projections,
  statuses, hashes, and artifact references. It is not an unbounded raw-output
  archive. Complete eligible payloads belong in the canonical Tool Artifact
  store, subject to its quotas, redaction, sensitivity, MIME, and retention
  rules.
- Replay determinism. A resumed or replayed run must assemble byte-identical
  history, so the reference form has to be reconstructible from persisted state
  rather than recomputed heuristically.
- `edit_file` preconditions. It requires a prior `observation_id` plus `version`;
  reference-form history must keep those reachable, not weaken them.

The split is deliberate: **the trace is the record, history is the working set.**
They serve different consumers and should not have the same contents.

## 4. Existing Reference Building Blocks

`runtime/src/environment.rs:1661` defines `Observation`, and it already carries
every field a reference needs:

```rust
pub struct Observation {
    pub id: String,
    pub source: String,
    pub start: usize,
    pub end: usize,
    pub byte_count: usize,
    pub digest: String,
    pub version: String,
    pub truncated: bool,
    pub artifact_ref: Option<String>,
}
```

It derives `Serialize, Deserialize`, and `Observation::from_bytes` computes `id`
as a stable hash over `source|start|end|digest|version`. So the identity is
already content-addressed and already deterministic.

After full-delivery, this transient Observation metadata is only one building
block. Durable history references must use the canonical Tool Artifact contract,
while Observation IDs and versions continue to protect coding mutations and
deterministic continuation. The gap is not solved by serializing this in-memory
store into a second durable payload database.

## 5. Blocking Prerequisite: Canonical Artifact Resolution

The audited `ObservationStore` (`runtime/src/environment.rs:1718`) is:

```rust
pub struct ObservationStore {
    state: Arc<RwLock<ObservationState>>,
}
```

in-memory only, scoped to the Engine instance. That observation state remains
appropriate for coding mutation preconditions and transient projections; it is
not the place to add durable conversation payloads after full-delivery.

The prerequisite for history references is the completed canonical Tool Artifact
authority: an opaque `ArtifactRef`, a run/session binding, a durable payload
ledger, bounded quotas, and a safe resolution path after resume. If a file read
already has a durable artifact projection, the history design must reuse that
reference. If it does not, the design must either retain a bounded inline model
projection or fail explicitly; it must not create a dangling reference.

That makes durable artifact resolution a hard prerequisite, not a follow-up. A
reference in history that cannot be resolved after resume is worse than the
current duplication: today a resumed run has stale-but-present content, whereas
a dangling reference has nothing.

Ordering is therefore fixed:

1. Confirm the canonical Tool Artifact store persists eligible payloads and
   references through checkpoint, resume, repair, cleanup, API, Web, and
   Desktop projections.
2. Keep the current-round model projection available within the inline bound.
3. Only then replace repeated or older history payloads with deterministic
   summaries and ArtifactRef-backed references.

Attempting step 3 first produces a resume regression that existing identity-only
tests may not catch, because they do not prove that the model or UI can resolve
the referenced tool result.

## 6. Approach

### 6.1 History entry shape

A repeated or older tool result in history becomes the canonical artifact
reference plus a deterministic summary, rather than another full payload:

```
[tool] read_file Cargo.toml (bytes 0-1841, sha256:9c1f..., v=mtime:...)
       1841 bytes, 62 lines. Full eligible content available by artifact ref.
```

The exact rendering is a detail; the constraints are not.

### 6.2 The summary must be deterministic

Generated from file facts only - byte count, line count, range, truncation flag.
Never model-generated, and never a model-written abstract of the content.

A model-generated summary would break replay determinism, since the same trace
would reconstruct different history on different runs, and it would put an
unverified paraphrase into the audit-adjacent path. If the model needs a
condensed reading of a file, that is the model's own turn output, which is
already recorded as such.

### 6.3 Resolution path

The model needs a way to pull the full text of a reference it holds. Options, in
order of preference:

1. **Existing bounded tool read.** The model can use `read_file` with a range
   and the reference's source/version facts. This is preferred for workspace
   files, provided a stale version fails loudly.
2. **Artifact fetch projection.** If the source is no longer available, a
   bounded, permission-checked artifact resolution path may return the stored
   payload or preview. It must use the canonical Artifact authority and never
   follow remote URIs or turn a reference into a capability.

Option 1 also composes with the existing `edit_file` precondition, since both
speak in `source` + range + `version`.

### 6.4 Retention

References are cheap and can stay in history indefinitely. Payloads are bounded
by the canonical Tool Artifact quotas and retention policy. When a payload is
evicted, the reference must either resolve through the canonical versioned
source/artifact path or fail loudly rather than silently returning different
content. The model and UI must see that failure as a missing/expired artifact,
not as success.

### 6.5 Interaction with compaction

In the audited baseline, compaction summarizes history that already contains
full payloads.
Reference-form history is smaller, so compaction triggers later. That is the
intended benefit and needs no compaction change, but the compaction threshold
tests will shift and must be re-baselined rather than adjusted to keep old
numbers.

## 6.6 Adjacent finding: duplicated `call_id` in `tool_call_completed`

Noted during the same review, and in scope here because it touches the same
record shape.

`runtime/src/foundation/events.rs:59`:

```rust
ToolCallCompleted { call_id: CallId, result: ToolResult },
```

and `core/src/types.rs:90`:

```rust
pub struct ToolResult {
    pub call_id: CallId,
    pub output: String,
    ...
}
```

So every `tool_call_completed` line in `trace.jsonl` carries `call_id` twice - once
on the event and once nested inside `result`. Checked across the 26
`tool_call_completed` events in the existing runs: **zero** mismatches. The nested
copy has never carried information the outer one did not.

Two things follow, and they should not be conflated:

- The redundancy is real. The nested field is derivable from the event.
- Removing it is a **canonical event schema change**, which is a different class of
  change from everything else in this document. It affects trace parsing, the
  SQLite index, and any consumer that reads `result.call_id`.

Recommendation: do not fold it into the G-G work. `ToolResult` is also the type
whose shape G-G changes (payload to reference), and changing identity fields and
payload fields in the same pass makes a schema migration harder to review. Record
it, decide it separately, and if it is taken, take it as a deliberate schema
version bump with a migration path for existing traces rather than as cleanup.

The safe intermediate position, if the duplication is judged not worth a schema
bump: keep the field and add a test asserting the two are always equal, which turns
an invariant that currently happens to hold into one that is enforced.

## 7. Non-Goals

- No silent loss of trace facts. Trace projections remain bounded and may refer
  to canonical artifacts rather than embedding every payload byte.
- No canonical event schema change. Section 6.6 is recorded, not scheduled.
- No model-generated summaries anywhere in the reconstruction path.
- No content-similarity or embedding-based deduplication. Identity is the
  content-addressed `Observation::id`, nothing fuzzier.
- No cross-run observation sharing. Scope stays within a run's resumable state.
- No change to the approval gate. A reference is data, not a capability.

## 8. Acceptance Criteria

1. The evidence task's later-turn prompt cost is measurably lower for repeated
   reads without hiding the current-round content from the model.
2. Reading the same payload twice reuses one durable artifact while preserving
   both call-level provenance records.
3. Trace/audit records retain event facts, hashes, status, and artifact lineage;
   eligible payload bytes are verified in the artifact store rather than assumed
   to be present in trace.
4. A run that reads a file, is interrupted, and resumes can resolve every
   reference used by the model/UI or reports an explicit expired/missing state.
5. `edit_file` still rejects a stale `observation_id` + `version` pair with the
   same fail-closed behavior as today.
6. Replaying a recorded run twice produces byte-identical assembled history and
   provider projections.
7. `agent-smoke` results are recorded before and after under `.rove/bench` in the
   same format, with artifact evidence kept separate from generated state.
