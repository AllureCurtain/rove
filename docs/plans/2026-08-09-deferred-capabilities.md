# Deferred Capabilities

> Status: **Rebaselined deferred-capability record.** G-D and G-E remain
> explicitly deferred. G-F was deferred in the 2026-08-09 snapshot but is
> assumed completed by the full-delivery lifecycle work; its rationale is kept
> below as historical context. Use
> [`2026-08-10-post-full-delivery-productization.md`](2026-08-10-post-full-delivery-productization.md)
> for active work.
>
> Recorded so the boundary stays explicit. Not scheduled. Do not begin
> implementation from this document.

## G-D - Parallel Subagents (P1, deferred)

### G-D current state

Verified on 2026-08-09: `subagent` appears **nowhere** in first-party `main`
source. It appears only in archived design documents, and the active
implementation program explicitly forbids it as a working method:

> "Subagents are prohibited inside every implementation conversation."

That rule governs how we build rove. It is unrelated to whether rove should offer
subagents as a product capability. The two must not be conflated.

### G-D existing alternative

Batch-scoped parallelism. Within one model turn, multiple calls that are
non-destructive and `parallel_safe` run concurrently. Destructive, unknown,
shell, write, request-input, and memory-write calls serialize through the
approval and execution boundary. History and trace are written back in model call
order after the batch completes.

This is not DAG scheduling. The runtime does not infer dependencies between
arbitrary tool arguments. A call needing a prior result is issued on a later turn.

### G-D missing capability

- Parallel exploratory branches over independent hypotheses.
- Concurrent execution of independent subtasks.
- Context-isolated delegation, where intermediate output does not consume the
  parent context window.

The third item is the real lever. Claude Code's delegation model exists primarily
to keep broad search output out of the main context.

### G-D deferral rationale

It interacted with the kernel unification program (G-C). Under the assumed
full-delivery baseline, that dependency is resolved, but a subagent still needs
an isolation story for state and approval and a rule for how child events enter
the parent's canonical ledger. The Fork provenance model and the shared kernel
are the relevant precedents.

It remains deferred because it expands concurrency, approval, lineage, context,
and cost surfaces before the single-agent product path has been proven with real
repository workloads. The reason is product sequencing, not an unfinished
kernel.

### G-D future constraints

- A subagent must not receive broader permissions than its parent.
- Child events must have a defined relationship to the parent canonical stream.
  Fork's read-only-projection approach is the reference, not event copying.
- Approval must not be delegable. A child needing approval surfaces to the same
  human gate.
- Concurrency must not create a second path around the tool `Executor` pipeline.

---

## G-E - Execution Sandboxing (P1, deferred)

### G-E current state

Shell execution is bounded by **policy**, not isolation. `ShellPolicy` provides
`timeout_ms`, `max_output_bytes`, `inherit_environment`, and a `denylist`.
`LocalProcessHost` adds canonical path enforcement, cancellation, kill-and-wait
cleanup, and opaque background process identity.

There is no container, no seccomp filter, no filesystem namespace, no user
separation.

### G-E honest limitation

A denylist is a blocklist, and a blocklist is not complete in principle. Once a
user approves `run_shell`, the command executes with the full privileges of the
rove process. The real defenses are the approval gate, the workspace path
boundary, and auditability - not confinement.

This is a stated consequence of the local-first design, not an oversight. It is
already recorded in `README.md` under current boundaries.

### G-E deferral rationale

Sandboxing is a prerequisite for multi-tenant or remote execution, and neither is
on the roadmap. Adding it now would impose real cost - platform-specific code,
degraded tool capability, harder debugging - for a threat model we do not
currently serve.

### G-E prior art

PI's sandbox extension replaces the default `rg`/`fd` operations through a
`customOps` seam rather than confining the process. rove's
`ExecutionEnvironment` port already provides the structurally equivalent seam:
`LocalExecutionEnvironment` and `InMemoryExecutionEnvironment` are two
implementations of one trait. A future sandboxed adapter would be a third, which
means **the architecture is already prepared for this** even though the capability
is absent.

That is worth stating precisely: the gap is a missing adapter, not a missing
abstraction.

### G-E future constraints

- A new adapter, not a special case inside `LocalExecutionEnvironment`.
- Capability loss must be explicit typed unavailability, following the existing
  `process_pty: false` precedent, never silent degradation.
- Sandbox presence must not weaken approval gating. Confinement and consent are
  independent controls.

---

## G-F - Model-On-Ambiguity Plan Evaluation (P2, completed by full-delivery assumption)

### G-F completed-state interpretation

The 2026-08-09 snapshot described a deterministic provider-free evaluator in
`runtime/src/planning/plan_evaluator.rs`. Under the full-delivery planning
baseline, the lifecycle work has added bounded ambiguity evaluation, an
independent Finalizer, public budget surfaces, and trace-tail reconciliation.
The deterministic evaluator remains the default and fallback.

Handled statuses: `Succeeded`, `Skipped`, `Failed`, `Blocked`, `BudgetExhausted`,
`Cancelled`, `Interrupted`, `Partial`.

### G-F preserved strengths

- Predictable, testable, zero-cost.
- No provider dependency in the control path, so a provider outage cannot corrupt
  plan progression.
- Every terminal `step_result` yields exactly one correlated `plan_decision`,
  linked by `trigger_step_record_id`.

### G-F snapshot gap

Judgment on ambiguous outcomes. A test failing after a code change could mean the
change is wrong, or the test was already stale. A rule cannot distinguish these.
The evaluator then classified by status and recoverability only. That limitation
is retained here as historical rationale, not as a current-state claim.

### G-F historical deferral rationale

The current evaluator is not wrong, it is limited. Introducing a model into the
control path is a significant change: it adds latency and cost to every step
boundary, creates a failure mode where evaluation itself fails, and weakens the
determinism that the benchmark suite depends on.

Related items that were unstarted in the snapshot, now covered by the assumed
full-delivery lifecycle work:

- An independent Finalizer.
- Public multidimensional budget configuration.
- Global model/tool/token accounting with structured budget events.
- Reconciliation of canonical trace events newer than the latest `TaskState`
  snapshot.

### G-F completed-design constraints

- The deterministic evaluator stays the default and the fallback. Model
  evaluation is consulted only for genuinely ambiguous cases.
- A failed model evaluation must degrade to the rule result, following the
  compaction circuit-breaker precedent already in
  `runtime/src/context/compaction.rs`.
- The one-decision-per-terminal-record invariant is preserved.
- The deterministic benchmark path remains provider-free so `agent-smoke` keeps
  working with no network.
