# Benchmark Result Packages

Each benchmark run should write a dated directory:

```text
benchmarks/results/<scenario>-<YYYY-MM-DD>/
  DATA_PROVENANCE.md
  rove-benchmark-core-report.md
  metrics.json
  artifacts/
```

`DATA_PROVENANCE.md` records command lines, git commit, provider mode, model id,
whether network was used, workspace path, and whether artifacts contain secrets.

`rove-benchmark-core-report.md` summarizes:

- harness regression;
- context ablation;
- working memory ablation;
- recovery/resume ablation;
- provider behavior when a real provider was used;
- failures classified as model, provider, runtime, or harness.
