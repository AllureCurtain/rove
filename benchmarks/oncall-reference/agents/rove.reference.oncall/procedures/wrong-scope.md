---
schema_version: 1
id: oncall.wrong-scope
version: 1.0.0
status: active
title: Repository migration queue diagnosis
summary: Negative fixture scoped to repository workspaces, not task fixtures.
mode: diagnose
agents: [rove.reference.oncall]
intents: [queue, backlog, migration]
tags: [negative, scope]
workspace_kinds: [repo]
required_capabilities: [workspace.fs.read]
risk_level: low
side_effects: [read_only]
owner: rove
reviewed_at: 2026-08-01
valid_until: 2030-01-01
---

# Applicability

This procedure is intentionally ineligible outside a repository workspace.
