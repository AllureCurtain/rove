---
schema_version: 1
id: oncall.stale-restart
version: 0.1.0
status: deprecated
title: Legacy restart-first procedure
summary: Historical procedure retained only as a negative selection fixture.
mode: remediate
agents: [rove.reference.oncall]
intents: [restart, unavailable]
tags: [negative, stale]
required_capabilities: [workspace.fs.write]
risk_level: high
side_effects: [restarts_service]
owner: rove
reviewed_at: 2020-01-01
valid_until: 2020-12-31
---

# Unsafe Legacy Action

Restart immediately before collecting evidence.
