# Contributing to rove

> Status: **Current contributor guide**. This file owns the contribution
> workflow: branches, worktrees, commits, pull requests, review, and PR
> scope discipline. Repository-wide engineering rules stay in
> [`AGENTS.md`](AGENTS.md); current runtime behavior stays in
> [`docs/runtime/`](docs/runtime/README.md).

These rules apply to every change, including documentation-only changes.
They exist so that each change stays reviewable, verifiable, and
revertible on its own.

## 1. How the rules are organized

- `CONTRIBUTING.md` (this file) is canonical for the contribution
  workflow.
- [`AGENTS.md`](AGENTS.md) is canonical for architecture invariants,
  current implementation boundaries, editing rules, verification gates,
  documentation governance, the security checklist, and handoff
  requirements. It is the entry point that coding agents load.
- [`docs/runtime/`](docs/runtime/README.md) is canonical for current
  behavior, [`docs/design/`](docs/design/) for proposed targets, and
  [`docs/plans/`](docs/plans/) for implementation plans.
- Information precedence follows the source-of-truth list in
  [`AGENTS.md`](AGENTS.md).
- When two documents repeat the same long-form rule, keep the canonical
  copy and link to it. Do not maintain parallel versions.

## 2. Hard workflow rules

- **Never commit directly to `main`.** Every change lands through a pull
  request — features, fixes, refactors, dependency updates, and
  documentation alike.
- All changes happen in a git worktree on a feature branch created from
  the latest `origin/main`.
- Branch names follow `<type>/<topic>`: `feature/…`, `fix/…`, `docs/…`,
  matching the type used in commit subjects.
- Pull requests merge with a merge commit ("Create a merge commit"), so
  the granular commits inside a PR remain visible in `main` history.
- Generated artifacts and lockfiles are committed in the same PR as the
  change that produced them, never as stray diffs.

## 3. Worktree workflow

Worktrees live under `.worktrees/<topic>/` (already gitignored) so they
do not litter the repo root or collide with the main checkout:

```bash
git fetch origin main
git worktree add .worktrees/<topic> -b feature/<topic> origin/main
cd .worktrees/<topic>
```

- Branch from `origin/main`, not from a stale local `main`.
- Keep a long-running branch current by merging `main` into it. Rebase
  only branches that have never been pushed.
- After the PR merges, remove the worktree and delete the branch:

```bash
git worktree remove .worktrees/<topic>
git branch -d feature/<topic>
```

## 4. PR scope discipline

- **One PR is one bounded, independently usable, and verifiable
  capability.** A reviewer should be able to accept or reject it as a
  unit.
- State the PR's scope ceiling, exclusions, and stopping conditions in
  the description — the template's "Scope and non-goals" section —
  before or alongside the implementation.
- Combine closely related changes that share initialization or security
  boundaries. Separate work that carries independent risk, an unresolved
  design, or a different verification path.
- Unrelated refactors, formatting sweeps, and dependency updates get
  their own PRs.
- Do not let a functional batch grow without a stopping point. Reassess
  every new prerequisite against the stated scope before adding it.
- If review and fixes keep cycling without converging, stop patching and
  reassess the design. Document the unresolved problem and defer it
  instead of expanding the PR.
- Migration rule: do not split an already in-flight batch just to adopt
  this workflow. Apply it from the next new PR onward.

## 5. Commits

- Subjects follow `type(scope): subject`, with types `feat`, `fix`,
  `docs`, `refactor`, `test`, `build`, `ci`, `chore`, and a scope naming
  the affected area (`runtime`, `api`, `web`, `cli`, `scripts`, …).
  Match the surrounding history.
- Write the subject as the observable behavior or outcome, not a diff
  label: `fix(scripts): report real exit codes from product-acceptance
  on Windows PowerShell 5.1`, not "update script".
- One logical change per commit. Granular commits inside a PR are
  welcome — they stay visible through the merge commit and make review
  and revert cheaper.
- No secrets, no generated noise, and no `wip` commits at review time.

## 6. Review policy

Choose the review method by risk and record it in the PR description.
Required checks apply regardless of the method.

- **Self-review** is acceptable for small, verified changes with a
  single owner and no new or changed cross-package contract.
- **Independent blind review** for security- or safety-sensitive
  changes, shared runtime boundaries (events, state, approval, MCP,
  providers, artifacts), and uncertain cross-package behavior. After
  implementation and checks, give a fresh reviewer — for example a new
  agent session — only the requirements, acceptance criteria, project
  rules, scope boundaries, repository location, and comparison baseline.
  Do not provide the implementation narrative, the self-assessment, or
  suspected defects.
- Address in-scope blocking findings and rerun the relevant checks.
  Low-value or out-of-scope suggestions may be declined; record material
  deferrals in the PR description instead of expanding the scope.
- Never defer a safety, security, or data-consistency blocker while
  claiming the affected behavior passed.

Changes touching tools, API, providers, state, MCP, artifacts, or Web
walk the security checklist in [`AGENTS.md`](AGENTS.md) before handoff.

## 7. Verification

[`AGENTS.md`](AGENTS.md) and its Verification section own the canonical
verification gates and the opt-in classification of real-service checks.
Headlines:

- Run the smallest relevant check first and expand in proportion to
  risk.
- Reach local CI parity before pushing: run locally the same commands CI
  will run, so a failing push does not burn a round-trip.
- Never report a gate as passed without observing its real exit status.
  A skipped real-service test proves only the skip path.

## 8. Documentation in the same PR

- If a change creates, removes, or clarifies an architecture rule,
  ownership boundary, workflow, required check, or generated artifact
  contract, update the governing document in the same PR:
  `docs/runtime/` for current behavior, `AGENTS.md` for repo-wide rules,
  this file for the contribution workflow.
- Do not let code establish a convention that no document describes.
- A change to the contribution workflow itself updates this file in the
  same PR.
- Documentation-only changes follow the docs checklist in the
  Verification section of [`AGENTS.md`](AGENTS.md): relative links,
  balanced code fences, heading structure, trailing whitespace, and
  current/proposed wording.

## 9. Code comments

- Default to no comments. Add one only when the **why** is non-obvious:
  a hidden constraint, an invariant, a workaround for a specific bug, or
  behavior a reader would not expect.
- Do not explain *what* — identifiers carry that. Do not stamp the
  current task, caller, PR, or issue number; those live in the commit
  message and the PR description.
- Rust public-API doc comments stay concise; expand beyond a line or two
  only when the contract genuinely requires it.

## 10. Language

Code, comments, documentation, commit messages, PR descriptions, and
verification reports are written in English.
