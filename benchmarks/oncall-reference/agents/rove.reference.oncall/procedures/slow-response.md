---
schema_version: 1
id: oncall.slow-response
version: 1.0.0
status: active
title: Diagnose slow responses
summary: Correlate latency with database and external dependency evidence.
mode: diagnose
agents: [rove.reference.oncall]
intents: [slow, latency, database, timeout]
tags: [oncall, latency]
required_capabilities: [workspace.fs.read]
risk_level: low
side_effects: [read_only]
owner: rove
reviewed_at: 2026-08-01
valid_until: 2030-01-01
---

# Applicability

Use for elevated request latency.

# Evidence

Distinguish database pool saturation from external dependency timeout using
correlated, time-bounded evidence.

# Safety

Do not flush caches or shift traffic in diagnosis-only mode.

