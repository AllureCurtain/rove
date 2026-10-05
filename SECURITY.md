# Security Policy

## Supported versions

Only the latest `main` is supported. Fixes land on `main` and are picked up by rebuilding from source; there are no backport branches.

## Reporting a vulnerability

Report privately through GitHub's **Security → Report a vulnerability** (private vulnerability reporting) on this repository. Do not open a public issue for undisclosed vulnerabilities.

Please include:

- A description of the issue and its impact
- Steps to reproduce or a proof of concept
- The commit or version you tested against

## Scope notes for this project

Rove handles provider API keys and executes tools on the local machine. Particularly in scope:

- Provider credentials leaking into committed files, logs, `trace.jsonl`, benchmark evidence, API responses, or UI screenshots
- Tool execution escaping the resolved workspace or bypassing the safety/approval path
- Path handling that trusts input from a provider, server, or MCP response

Expected disclosure: an acknowledgement within a few days, a fix or mitigation plan agreed before public disclosure.
