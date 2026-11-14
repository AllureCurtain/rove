# Pilot 2 — task: workspace + fork isolation

Pre-registered: 2026-11-14.

- Expectation: (a) a second workspace's session sees only its own root;
  (b) a fork inherits parent history but runs independently.

## Actual

**(a) Workspace isolation.** Created `pilot2-other` (`/tmp/rove-pilot2-other`,
one `notes.txt`). Granting all capabilities at creation did **not** cover
`provider_credentials`: setting the session's model config changes the
provider selector digest, which invalidated `provider_credentials` —
`invalidated_capabilities: ["provider_credentials"]` surfaced on the next
trust GET, and the queued follow-up exhausted its 4 start attempts with
`project_trust_required`, marking the session `needs_attention`. This is the
known trust-invalidation trap exercising the stranded-queue path. Re-granting
trust + `POST /controls/{id}/confirm` recovered it. The run then reported
correctly: `calc.py` absent, `pwd` = `/private/tmp/rove-pilot2-other`, only
`notes.txt` (18 B) — no read of the sibling calc repo.

**(b) Fork isolation.** `POST /sessions/{main}/forks` at run
`01M7CWG23FRWXZ6PVF1P86A6F0` → child `01M7CWHQQ89D87ABN84RWVT6JV`,
`fork_point_seq 2813`. Child transcript inherits all 7 parent segments with
`inherited: true` (including the cancelled refactor and the rejected-delete
error run). Sent fork-only "7 % 3?" message → child ran it (505 events,
`final`, answer "1") while the parent's transcript stayed at 7 segments —
no cross-leak. Note: parent session was `needs_attention` after the denied
delete; fork refused until the abandoned no-op control was confirmed
(`product_fork_source_invalid` "idle + final durable run"), then refused
again on run 6 with "runtime job identity does not match its product
binding" — succeeded only on the newest run. The mid-history fork refusal
may be a defect worth a follow-up (fork-at-older-run should be legal per
the request schema); recorded here for triage.

## Verify (independent)

- ws2 run answer states workspace contains only `notes.txt`; `run_shell`
  `pwd` output `/private/tmp/rove-pilot2-other`.
- Fork `fork_point_seq 2813` == parent's last completed run's final seq.
- Parent transcript still 7 segments after child's new run.

## Verdict

**PASS** — workspace boundary and fork isolation both hold; the
trust-invalidation → needs_attention → recover chain was exercised for real.
One follow-up candidate: fork refused at a mid-history `done` run
(`01M7CVVDTY3KB3MBHKCDC28EA8`) with a binding-identity error — needs a
repro test to decide whether it's a defect or correct strictness.
