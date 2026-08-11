---
schema_version: 1
id: oncall.memory-high
version: 1.0.0
status: active
title: Diagnose memory growth
summary: Distinguish a sustained leak from a transient allocation spike.
mode: diagnose
agents: [rove.reference.oncall]
intents: [memory, leak, rss, heap]
tags: [oncall, memory]
required_capabilities: [workspace.fs.read]
risk_level: low
side_effects: [read_only]
owner: rove
reviewed_at: 2026-08-01
valid_until: 2030-01-01
---

# Applicability

Use for sustained memory growth. A single high sample is not sufficient.

# Evidence

Compare a bounded trend with allocation or restart evidence and explicitly
record whether an OOM event exists.

# Safety

Do not force a restart or dump potentially sensitive process memory.

