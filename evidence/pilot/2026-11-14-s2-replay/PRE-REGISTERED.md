# Pre-registered expected outcome (written BEFORE the agent run)

Pilot task: S2 regression-replay
Date: 2026-11-14
Operator: ego-lite (DeepSeek `deepseek-v4-pro` via web console)

## Environment
- Product under test: rove main `d8aec71` (`target/debug/rove-api` + `apps/web/web-dist`, built ~12:01), port 18789, config `/tmp/rove-pilot2-config`, data `/tmp/rove-pilot2-data`.
- Target the agent edits: rove checkout at pre-fix commit `ea65371` (parent of real fix `31cbfdb`) at `/tmp/rove-replay-s2`.
- Confirmed bug present: `apps/api/src/product/transcript/reader.rs` has `MAX_EVENTS_PER_RUN: usize = 2_000` (line 31) and `MAX_TOTAL_EVENTS: usize = 10_000` (line 32).

## Prompt sent to agent
> In this repo, loading a session transcript for a run that produced many events (e.g. one that fanned out subagent events) gets truncated: the response stops at a `response_limit_reached` partial reason partway through the run with no cursor to continue from, so the transcript looks cut off. Find where the per-run transcript event limit is enforced and fix it so a run with many events isn't truncated prematurely. Explain what you changed.

## Expected
The real upstream fix `31cbfdb` changed `MAX_EVENTS_PER_RUN` 2000→20000 and `MAX_TOTAL_EVENTS` 10000→25000 with a comment in `apps/api/src/product/transcript/reader.rs`.

A PASS run: agent locates `apps/api/src/product/transcript/reader.rs`, identifies the per-run event cap (`MAX_EVENTS_PER_RUN` / `response_limit_reached` partial reason) as the cause of premature truncation, and raises it sensibly (and/or adds a continuation/cursor). Exact numbers need not match 20000/25000 — correct diagnosis + sensible raise + explanation counts as PASS on correctness of intent.

FAIL: wrong file, wrong cause, broken code (doesn't compile), unrelated churn, or no edit produced.
