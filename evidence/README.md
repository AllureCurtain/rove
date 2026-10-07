# Evidence

Curated engineering-validation artifacts for the external control surface
(`rove-api` + `apps/web`). This directory is committed to git; bulky or
intermediate output stays in the gitignored `/outputs/` (or
`.rove/acceptance-logs/` for raw check output).

## What counts as an acceptance run

One acceptance run executes `scripts/product-acceptance.ps1` to completion and
records its report here. The script refuses to report a pass without a real
exit code, and gated checks are recorded as `not_run` with their reason — a
PASS verdict therefore means every required gate actually ran.

Run it from a clean checkout at the commit being validated:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/product-acceptance.ps1 `
    -ReportPath "evidence/acceptance/<YYYY-MM-DD>-<short-sha>/PRODUCT_ACCEPTANCE_REPORT.json"
```

Browser e2e runs are part of the required set. Run without `-SkipBrowser`
unless browsers are genuinely unavailable, and note any skipped check in the
record.

## Layout

```
evidence/
  acceptance/
    <YYYY-MM-DD>-<short-sha>/
      ACCEPTANCE.md                     human record: scope, environment, verdict, notes
      PRODUCT_ACCEPTANCE_REPORT.json    verbatim output of the acceptance script
      ...                               optional curated extras (traces, screenshots)
```

- Directory name: run date plus the short commit SHA under test.
- `ACCEPTANCE.md` states what was validated (fake-model pipeline vs.
  credentialed real provider), the host OS/toolchain, the verdict, and links
  every extra file to what it demonstrates.
- Screenshots are listed in `ACCEPTANCE.md` with a one-line caption each.

## Hygiene rules — read before committing

- **No secrets.** Provider keys, tokens, and credentials must never appear in
  any artifact. The runtime redaction layer covers traces, but JSON reports
  and screenshots need a manual once-over (`Select-String -Pattern "sk-"`,
  check visible URLs and terminal output in screenshots).
- **No private paths that expose more than the repo already does.** User
  home directories in logs are acceptable on a personal dev machine; customer
  or third-party paths are not.
- Reports are copied verbatim — do not edit a report to make a run look
  greener; a FAIL verdict is committed as FAIL if the artifact is kept at all.
- Keep it curated: one directory per acceptance run, not per check.
