# Engineering validation and delivery

Updated: 2026-11-09. This is a validation plan, not a completed deployment report.

Rove targets a developer working on a local repository: describe a task, inspect the proposed actions, authorize changes, review the result, and continue after interruptions. The smallest deliverable is the hosted browser console with the shared Rust runtime. Other surfaces share that runtime but need their own installation and interaction validation.

## Acceptance boundary

A release candidate is ready for a supervised pilot when all required automated acceptance gates pass, one supported installation path works on a clean machine, and credentialed tasks succeed on disposable copies of representative repositories. Model-list connectivity alone does not establish task completion. The deterministic fake model proves orchestration and UI contracts; it does not prove natural-language programming quality.

Use the same commit and record the OS, Rust/Node/pnpm versions, browser build, provider wire protocol and model. Record prompts, fixture commit, expected result, actual outcome, elapsed time, usage, intervention count and defects. Keep credentials, personal paths and private project contents out of committed evidence. Preserve failed runs and skipped gates with their reasons.

## Task matrix

| Scenario | User action | Acceptance evidence |
| --- | --- | --- |
| First launch | Start the supported build, open a disposable repository, configure a real model | Reachable URL, readable setup errors, no false readiness in demo mode, successful task beyond catalog test |
| Understand code | Ask where a known behavior is implemented | Correct file/line references checked against the fixture; no writes |
| Make a bounded change | Request a specific small bug fix with a known failing test | Reviewed diff, independent test result, unchanged unrelated files |
| Approval | Approve one write and deny another | Approved bytes match; denied write is absent; no automatic permission escalation |
| Continue | Send a second related task, queue another during a run | Correct session and repository; accepted message executed once; independent verification of result |
| Cancel | Stop during a model/tool wait, then continue | Terminal state stays stopped; partial output identified; no repeated completed write |
| Refresh/restart | Refresh while waiting for approval; restart a supported host | Same pending action or explicit safe recovery; no silent resend or duplicate change |
| Duplicate launch | Open a second host on the same data root during a pending approval | Second host refused before recovery; original task and approval unchanged; crash release permits explicit recovery |
| Inspect/revert | Open changed file and diff; revert one unchanged recorded mutation | Exact original bytes restored; conflicting later edits refused |
| Isolate | Switch between two repositories and fork a completed conversation | Correct model, trust prompt and transcript per session; child continuation independent |
| Long history | Read earlier turns, load additional pages, return to latest | Stable reading position, bounded rendered window, user scrolling takes precedence |
| Failure | Invalid credential, unavailable model, provider error, tool failure | Actionable error, safe credential handling, no fabricated success or repeated side effect |

## Pilot record

For each real engineering trial, fill in this record from observed facts. Do not use automated fixtures as a substitute for an actual project trial.

- Trial ID, date, operator and environment:
- Real project and reason this task matters (sanitized identifier):
- Repository baseline and Rove commit:
- Provider/model and task prompt (no credentials):
- Expected outcome and independent acceptance check:
- Actual outcome, time/usage and manual interventions:
- Files changed, tests run and review/revert evidence:
- Defects, mitigation and follow-up issue/PR:
- Practitioner feedback and whether the result was accepted:

Aggregate success counts with a denominator and retain failed attempts. Compare against a documented baseline only when the same task and conditions were measured. Do not claim productivity gains from subjective impressions or deterministic demo results.

## Documentation package

| Material | Existing source | Work still needed |
| --- | --- | --- |
| Requirements and scope | This document, `TODO.md`, `docs/web-console-design.md` | Confirm target users, supported platform and pilot scenarios |
| System design | `docs/architecture.md`, `docs/decisions.md` | Maintain alignment with the delivered commit |
| Interface and data contracts | `docs/api.md`, generated OpenAPI, schema migrations | Freeze the release version and link key lifecycle/recovery decisions |
| Development and deployment | `docs/development.md`, build and acceptance scripts | Clean-machine installation record and upgrade/recovery exercise |
| User operations | `docs/user-guide.md` | First-time-user walkthrough and observed issues |
| Testing | `evidence/acceptance/`, regression tests, benchmark definitions | Credentialed task results and real engineering pilot report |
| Contribution | Git history, issues/PRs and design records | Identify the applicant's own design, implementation and validation work |

## Degree evidence mapping

The supplied 2026 electronic-information degree standard, page 7, innovation criterion 8 requires applying for software copyright **and** a functionally complete system validated in actual engineering practice, with complete design and development documents. The user reports that copyright has been obtained; the certificate and its relationship to the delivered software version have not been inspected here.

Copyright documentation, system acceptance, real engineering trial evidence and the design/development package are distinct records. The standard also requires at least six months of professional practice (pages 4–5), a compliant thesis or practice report, and a clear account of individual contributions for team work (page 7). This technical plan does not establish that those separate requirements have been met. The standard refers to additional university and engineering-degree rules which are not included in the supplied file.
