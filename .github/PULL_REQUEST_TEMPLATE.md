<!--
One PR is one bounded, independently verifiable capability
(CONTRIBUTING.md §4). Trim sections that do not apply.
-->

## Summary

<!-- What does this change, and why does it matter? -->

## Scope and non-goals

<!-- The ceiling of this PR: what it deliberately does not do. Deferred
     work belongs in follow-up PRs, not here. -->

## Implementation notes

<!-- Trade-offs, alternatives considered, anything subtle a reviewer
     should know. Skip if the diff is self-explanatory. -->

## Verification

<!-- Check only what you actually ran, with real exit codes. -->

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] Web (`apps/web/`): `pnpm test`, `pnpm typecheck`, `pnpm build`
- [ ] Focused integration tests (if affected): `cargo test -p rove-integration-tests --test …`
- [ ] `pnpm test:e2e` (browser-visible flows, SSE, approval/input/cancel/resume, or API proxy changes)
- [ ] Docs updated in the same PR where the contract changed (`docs/runtime/` for current behavior; `AGENTS.md` / `CONTRIBUTING.md` for workflow or boundary rules)

## Review

- Review method: <!-- self-review / independent blind review -->
- Deferred follow-ups: <!-- or "none" -->

## Notes for reviewers

<!-- Optional: areas that want extra scrutiny. -->
