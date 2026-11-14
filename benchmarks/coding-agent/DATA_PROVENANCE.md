# Coding-Agent Benchmark V2 — Data Provenance

Deterministic, network-free evaluation of the rove coding-agent core loop. Five
scripted scenarios run on the `fake_contract` provider profile against a local
direct transport; every scenario is gated by hard oracles
(`fixture_ledger_matches`, `terminal_status`, `no_duplicate_external_effect`,
plus per-case capability/mutation assertions). No real credentials are used and
no network access is required.

## Run it

```bash
cargo run -p rove-bench -- \
  --suite-v2 benchmarks/coding-agent/suite.json \
  --output-dir benchmarks/results
```

The command exits non-zero if any hard oracle or quality gate fails. On success
it writes an evidence package under
`benchmarks/results/<UTC-stamp>-coding-agent-v2/` containing:

- `manifest.json`, `aggregate.json`, `summary.md`, `DATA_PROVENANCE.md`
- `scenarios/<case-hash>/` — `runtime/report.json`, `runtime/trace.jsonl`,
  `fixture/truth.json`, `fixture/request-ledger.jsonl`, `oracles.json`,
  `metrics.json`, `oracle-input.json`, `fixture-hash.json`

The fixture `request_ledger` in each `fixtures/*.json` is the independently
authored truth; the runner replays observed `tool_call_*` trace events into a
ledger and `fixture_ledger_matches` compares the two, so a model answer cannot
manufacture the oracle input.

## Scenarios

| ID | What it proves |
| --- | --- |
| K01-read-edit-verify | `read_file` → observation/version-guarded `edit_file` → `run_shell` verify terminates `final` with the exact mutation `update:src/lib.rs` recorded once. |
| K02-search-read-edit | `search_code` → `read_file` → scoped `edit_file` touches only `src/util.rs`; unrelated files are byte-identical (`expected_files`). |
| K03-failure-recovery | A `read_file` on a missing path fails (`error:execution_failed`), the agent recovers to the real target and completes the edit. |
| K04-cancel-resume | `cancel_run` after the second model turn cancels after a completed `write_file`; resume finishes `final` and the `create:notes.txt` mutation is not replayed (`no_duplicate_external_effect`, `resumed=true`). |
| K05-mutation-rejected | `reject_all_mutation` approval blocks `write_file` (`error:permission_denied`); `tool_mutations` stays `[]` and the file is never created. |

## Provenance fields

- Schema: `2` (`agent_evaluation`)
- Provider profiles: `fake_contract`
- Network: disabled
- Redaction: `fixture_and_runtime_scan`
- Git commit / dirty flag: recorded in `manifest.json` per run
