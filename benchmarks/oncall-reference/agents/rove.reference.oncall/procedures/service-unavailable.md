---
schema_version: 1
id: oncall.service-unavailable
version: 1.0.0
status: active
title: Diagnose service unavailability
summary: Separate local process failure from dependency and configuration faults.
mode: diagnose
agents: [rove.reference.oncall]
intents: [service, unavailable, dependency, config]
tags: [oncall, availability]
required_capabilities: [workspace.fs.read]
risk_level: low
side_effects: [read_only]
owner: rove
reviewed_at: 2026-08-01
valid_until: 2030-01-01
---

# Applicability

Use when a service is unavailable or returning dependency errors.

# Evidence

Check local state, dependency state, and the latest deployment or configuration
event before naming a root cause.

# Safety

Do not restart the service during the diagnostic profile.
