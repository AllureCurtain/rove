# Pilot 2 — task: understand code (scratch repo)

Pre-registered: 2026-11-14. Second real-project pilot on a disposable repo
(`/tmp/rove-pilot2-repo`, Python calc module with a deliberate `subtract` bug).

- Environment: `rove-api` on 127.0.0.1:18793, `ROVE_CONFIG_ROOT=/tmp/rove-pilot2-config`,
  `ROVE_DATA_ROOT=/tmp/rove-pilot2-data`; workspace `01M7CTZASWJ0VFSKJZ7ZDZGRPR`
  (repo kind, all capabilities granted + provider-credentials authorized via UI);
  session `01M7CTZD8N7FRPA1HF6TRMZV0B`, model `deepseek-flash`.
- Expectation: agent reads the workspace, identifies that subtraction lives in
  `calc.py::subtract`, and reports file + function without modifying files.

## Actual

- Run `01M7CV1VH0QB9QSHJQGVNK5X66`, status `done`, 6,436 events, ~11.8k ctx tokens.
- Plan of 4 steps (map repo → search identifiers → read files → report).
- Tools used: `repository_map` ×1, `list_directory` ×2, `glob_paths` ×1,
  `search_code` ×4, `read_file` ×5, `run_shell` ×4.
- Final answer (verbatim, abridged): "File path: `calc.py` (workspace root);
  Function name: `subtract(a, b)`" — correct.
- 3 sequential `run_shell` approvals were required mid-run (sha256sum,
  `python3 -c "import calc"`, `stat` verification). Each approval surfaced in
  the UI as a blocking card with 批准/拒绝 and resumed the run on approve.

## Verify (independent)

- `git status --porcelain` in the scratch repo shows only `?? __pycache__/`
  — a bytecode-cache artifact the agent itself disclosed in its reasoning
  ("importing calc would write `__pycache__/calc.cpython-*.pyc`"). Zero source
  modifications; `git diff` empty.
- Transcript `mutations: 0` — no file-mutation events recorded.

## Verdict

**PASS** — correct answer, no source modification, approvals gated and
resumable. Observation carried over from pilot 1 (defect #7, model behavior):
`deepseek-flash` over-verifies — it chained 4 approval-gated `run_shell` calls
to *prove* non-modification after the read tools had already answered the
question. Not a rove defect; noted for model-selection guidance.
