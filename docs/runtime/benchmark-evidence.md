# Benchmark Evidence

Benchmark claims must be backed by a result package under `benchmarks/results/`.

Use deterministic fake-provider runs for runtime regressions. Use real-provider
runs only for provider claims, and keep those claims separate from local runtime
health.

Required files:

- `DATA_PROVENANCE.md`
- `rove-benchmark-core-report.md`
- `metrics.json`

The report should follow pico's latest evidence shape while using rove
terminology: harness regression, context ablation, memory ablation,
recovery/resume ablation, and provider gate evidence.
