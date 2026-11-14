# Pilot S5 — inspect-and-revert matrix slice vs real provider

- Date: 2026-11-14
- Operator: ego-lite (ego-browser v2.2.1) driving the web console
- rove: main `6693bbb` (128k context, `max_model_turns_per_step` 48), `target/debug/rove-api`, `apps/web/web-dist`
- Provider: DeepSeek `deepseek-flash` via `https://api.deepseek.com` (env `DEEPSEEK_API_KEY`)
- Server: `127.0.0.1:18794`, `ROVE_CONFIG_ROOT=/tmp/rove-pilot5-config`, `ROVE_DATA_ROOT=/tmp/rove-pilot5-data`
- Target workspace: `/tmp/rove-pilot5-target` (disposable git repo, baseline `f533641` = README.md + greet.py)
- Session: `01M7CDGFC25RFKXNJ5P0Y10K71`, run `01M7CDGTY2X1YXSGHYH94K7AYV`

## Pre-registered expectations

- Change applied under approval; change card shows correct diff.
- Revert restores exact pre-change bytes; `git status` clean vs baseline.
- A conflicting re-revert / tampered file is refused, not clobbered.
- PASS = apply→inspect→revert round-trip restores byte-exact original; boundary refusal behaves.

## What was done

1. **Setup** — added provider via Settings → 模型服务 (name DeepSeek, type OpenAI,
   base `https://api.deepseek.com`, model `deepseek-flash`, env var `DEEPSEEK_API_KEY`);
   "密钥已验证并安全保存". Opened workspace `/tmp/rove-pilot5-target` (type 仓库),
   granted `provider_credentials` trust. Evidence: `01-setup-authorized.png`.

2. **Bounded change** — sent one message asking for two edits: add a one-line
   description to README.md and change `greeting()` in greet.py from
   `"hello, world"` to `"hello, rove"`. Two `edit_file` approvals appeared
   ("destructive tool requires explicit approval"); both approved.
   Evidence: `02-approval-edit-file.png`. Result recorded by agent:
   README 94→165 B (+1 line), greet.py 93→92 B (`hello, world`→`hello, rove`).
   Agent then ran redundant verification steps; the turn was stopped manually
   after the edits were already confirmed (deepseek-flash over-verifies on the
   4-step plan: 15 actions / 85.8k tokens for a 2-line change).

3. **Inspect the change** — the work panel 变更 tab listed the changed files
   (greet.py update, README.md update) and the 差异 section showed
   "4 处更改 +6 −2": two `run · update` mutation records (the recorded diffs)
   plus two `git · modified` records (the live worktree diff). Expanding the
   cards showed the exact hunks: README `@@ -1,3 +1,5 @@` +2/−0 and greet.py
   `@@ -1,5 +1,5 @@` −`hello, world` / +`hello, rove` +1/−1. Independent
   `git -C /tmp/rove-pilot5-target diff` produced the identical patch
   (`agent-change.patch`). Evidence: `03-change-diff-panel.png`.

4. **Revert** — the turn-change rollback button could not be exercised through
   the pointer because the transcript projection had dropped the `edit_file`
   tool-call cards (recovery notice: "the transcript response reached its
   safety limit. Expected event 2001, observed 4067"); the cards never
   re-rendered after reload/recovery-retry, so no `.change-card__rollback`
   node existed in the DOM. The same product revert path the button calls was
   therefore driven through the API:
   `POST /product/sessions/{id}/mutations/revert` with `{"path":"greet.py"}`
   then `{"path":"README.md"}`. Both returned
   `{reverted_operation:"update", action:"reverse_patched"}`.

5. **Post-revert state** — `git status --porcelain` empty, `git diff` empty:
   workspace byte-exact at baseline `f533641`. greet.py reads
   `return "hello, world"`; README.md is back to the 3-line original. The 差异
   count dropped to "2 处更改 +3 −1" (only the recorded run mutations remain as
   history; the live `git` diff entries vanished). Evidence:
   `04-post-revert-clean.png`.

## Boundary checks

- Re-revert `greet.py` after it was already reverted → **HTTP 409**
  `{"code":"revert_conflict","error":"the file no longer matches the recorded mutation"}`.
- Re-revert `README.md` → **HTTP 409** `revert_conflict`.
- Revert a path with no recorded mutation (`nonexistent.py`) → **HTTP 404**
  `mutation_not_found`. No clobbering in any case.

## Result

| Expectation | Actual | Verdict |
| --- | --- | --- |
| Change applied under approval | 2 `edit_file` approvals granted; diffs applied (README +1 line, greet.py string swap) | PASS |
| Change card shows correct diff | 差异 panel: 4 处更改 +6 −2, run+git records, hunks match independent `git diff` | PASS |
| Revert restores byte-exact original | both files `reverse_patched`; `git status`/`diff` clean vs baseline `f533641` | PASS |
| Conflicting re-revert / unknown path refused | 409 `revert_conflict` on re-revert; 404 `mutation_not_found` on unknown path | PASS |
| Change-card rollback button via pointer | not reachable — transcript dropped edit_file cards (event gap 2001→4067); API path used instead | PARTIAL (UI affordance not exercised) |

## Verdict

**PASS** — apply → inspect → revert round-trip restored the byte-exact original
(`git status` clean vs baseline), and the boundary refusals behaved correctly
(409 conflict / 404 not-found, no clobber).

One UI caveat: the change-card rollback button was not click-reachable because
the transcript's tool-call projection truncated (event 2001→4067 gap) and never
re-rendered the `edit_file` cards; the equivalent product revert endpoint was
exercised directly. This transcript-projection gap is a separate follow-up, not
a revert-mechanics failure.
