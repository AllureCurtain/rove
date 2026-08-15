# Benchmark Evidence

Benchmark claims must be backed by a result package under `benchmarks/results/`.

Use deterministic fake-provider runs for runtime regressions. Use real-provider
runs only for provider claims, and keep those claims separate from local runtime
health.

Required files:

- `DATA_PROVENANCE.md`
- `rove-benchmark-core-report.md`
- `metrics.json`

The productization B/C/D evidence boundary is explicit. Fake-provider runs are
the reproducible local-runtime evidence and may cover parser recovery, prompt
identity, traversal, context projection, artifact dedupe, resume, and replay.
Native-provider results must be recorded in a separate provider-gate package;
no native external-provider credentials or interoperability gate was run for
this integration, so that result is `unverified`, not a pass. The existing OnCall V2
suite remains an independent truth/safety-gated fake-provider evaluation and
must not be merged into a native-provider claim.

Recommended local commands:

```text
cargo run -p rove-bench -- --suite benchmarks/agent-smoke.json --output-dir .rove/bench
cargo test -p rove-integration-tests --test bench oncall_benchmark_v2_passes_independent_truth_and_hard_safety_gates -- --exact
cargo test -p rove-runtime --test tool_contract
```

Do not commit `.rove/bench`, `target`, provider keys, or raw provider output.

The report should follow pico's latest evidence shape while using rove
terminology: harness regression, context ablation, memory ablation,
recovery/resume ablation, and provider gate evidence.
