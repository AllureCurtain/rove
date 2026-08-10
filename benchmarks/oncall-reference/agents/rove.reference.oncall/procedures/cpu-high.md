---
schema_version: 1
id: oncall.cpu-high
version: 1.0.0
status: active
title: Diagnose sustained CPU saturation
summary: Correlate CPU saturation with process and deployment evidence.
mode: diagnose
agents: [rove.reference.oncall]
intents: [cpu, saturation, deployment, worker]
tags: [oncall, cpu]
required_capabilities: [workspace.fs.read]
risk_level: low
side_effects: [read_only]
owner: rove
reviewed_at: 2026-08-01
valid_until: 2030-01-01
---

# Applicability

Use for sustained CPU symptoms. Do not infer a runaway worker from CPU alone.

# Evidence

Confirm the time window, the dominant process, and a correlated deployment or
batch event. Cite only evidence IDs returned by the observation tool.

# Safety

Do not kill processes or restart services during diagnosis.

