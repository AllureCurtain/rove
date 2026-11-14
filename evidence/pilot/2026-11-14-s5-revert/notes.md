# S5 operator notes

## Environment
- Port 18794; isolated roots under /tmp/rove-pilot5-*; no ROVE_* budget overrides.
- deepseek-flash confirmed cheap but token-hungry on verification: the 2-line
  change burned 15 actions / ~85.8k tokens because the model re-read and re-ran
  `git status`/`git diff`/`grep -c`/`python3 greet.py` on every plan step and
  then attempted to re-do already-satisfied steps. Turn had to be stopped
  manually — worth noting for benchmark cost on the cheap model.

## UI path taken
- Settings → 模型服务: provider form. The `API 密钥` field is uncontrolled;
  `page.fill` did not take — used JS `value` setter + `input`/`change` events.
  Same for `密钥环境变量名` → `DEEPSEEK_API_KEY`. First save landed cleanly here
  (status "密钥已验证并安全保存"), no false-failure toast this run.
- Workspace: 手动输入路径 → absolute path → 类型 仓库. Trust alert "需要工作区授权"
  → inline 授权 → `provider_credentials`. Resolved path shows as
  `/private/tmp/rove-pilot5-target` (macOS /var↔/private/var canonicalization).
- Change inspection: 详情 panel → 变更 tab → 差异 section. "本次改动的文件" lists
  files; clicking one opens a `file:`-style read tab, not the rollback card.
  The 差异 cards expand to inline unified diffs (+/− counts, @@ hunks).

## Where the pointer path broke
- The transcript collapsed each model turn's tool calls behind "工具调用 × N"
  groups. The `edit_file` calls (which carry the rollback change cards) sat in a
  group that, once the turn was stopped, was covered by a transcript-recovery
  notice: "run 1 (01M7CDGTY2X1): the transcript response reached its safety
  limit. Expected event 2001, observed 4067." → the edit_file tool-call cards
  (and thus `.change-card__rollback`) never rendered, even after 重试恢复 and a
  full page reload. Only the early `read_file` group re-projected.
- Consequence: rollback had to be driven through
  `POST /product/sessions/{id}/mutations/revert` directly — same endpoint the
  card button (`change-card__rollback` → `onRevertMutation(path)`) invokes.
- Follow-up worth filing: transcript projection truncating mid-run drops the
  very tool-call cards that carry the revert affordance; the rollback UI is only
  as available as the projection.

## Revert semantics observed
- `update` mutation → `action: "reverse_patched"` (git apply -R after --check).
- Already-reverted path → 409 `revert_conflict` ("file no longer matches the
  recorded mutation") — mutation record is consumed/guarded, not re-applied.
- Unknown path → 404 `mutation_not_found`.
- The `run`-source diff entries persist after revert (history), while `git`-
  source entries disappear once the worktree is clean — 差异 count went
  4 处更改 +6 −2 → 2 处更改 +3 −1.

## Independent verification (not the agent)
- `git -C /tmp/rove-pilot5-target status --porcelain` → empty post-revert.
- `git diff` vs baseline `f533641` → empty; README.md and greet.py byte-identical
  to the committed originals.
- `POST /product/sessions/{id}/export` captured (session-export.json, 391 KB);
  transcript events carry `result.mutations`/`record.mutations` with
  `after_version` sha256 for both files.
