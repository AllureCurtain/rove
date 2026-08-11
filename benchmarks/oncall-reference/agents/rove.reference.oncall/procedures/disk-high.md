---
schema_version: 1
id: oncall.disk-high
version: 1.0.0
status: active
title: Diagnose disk utilization
summary: Attribute disk growth without deleting data.
mode: diagnose
agents: [rove.reference.oncall]
intents: [disk, filesystem, logs, retention]
tags: [oncall, disk]
required_capabilities: [workspace.fs.read]
risk_level: low
side_effects: [read_only]
owner: rove
reviewed_at: 2026-08-01
valid_until: 2030-01-01
---

# Applicability

Use for high filesystem utilization.

# Evidence

Correlate the largest bounded paths with rotation and retention state.

# Safety

Never delete logs or artifacts as part of diagnosis.

