# Desktop plan — making the Tauri host a real product

Status: design agreed 2026-11-14; implementation not started. Owner runs
platform-specific acceptance; everything else is normal branch+PR work.

## What exists today (do not re-scope)

`apps/desktop` is a **thin Tauri 2 host**, not a second app. It embeds the
real `rove-api` in-process (random `127.0.0.1` port), injects
`__ROVE_TOKEN__`/`__ROVE_API_URL__`/`__ROVE_CREDENTIAL_ENTRY__` into the
WebView, and loads the **same** `apps/web` build (`desktop-dist`) the
browser console uses. `apps/web/AGENTS.md` makes this a rule: *"a separate
Desktop-only UI is not allowed."*

Already implemented and tested:

- Single-instance ownership (`ApiOwnershipError::Busy` → native dialog,
  actionable text, no payload leak).
- Graceful exit: `ExitGate` drains the embedded API (pending approvals land,
  no half-written files) before the process dies.
- Windows native credential prompt (`CredUIPromptForCredentialsW`); the
  secret never crosses the JS boundary. macOS/Linux use the loopback
  onboarding form instead (verified end-to-end in the pilot).
- Native folder picker (`rfd`), bounded `show_in_folder` and `open_external`
  (workspace-root allowlist + http/https allowlist).
- `tauri.conf.json`: identifier `com.rove.agent`, CSP, NSIS perMachine,
  full icon set (`icons/` has icns/ico/png + Square logos).
- Window-state persistence (`window-state.json` under the config dir) and
  the platform app menu (PR #100).
- System tray with Show/Quit menu; a window close defaults to hiding to
  the tray and Settings → General can switch it to Quit — the choice lives
  in `desktop.json` (`close_action`) and stays a host-local concern.
- CI: `.github/workflows/desktop.yml` runs clippy+test on Ubuntu when the
  crate or workspace manifest changes.

The Web side already detects the host (`desktopTransport()`,
`platform/desktop-commands.ts`) and routes workspace-pick / credential /
external-link / reveal calls through `invoke` with boundary validation —
that channel plus `WorkspaceRoots` is the security contract; new host
features must keep it.

## What is actually missing

Grouped by who can do it.

### Can be built anywhere (feature work, normal PRs)

| Item | Why | Effort |
| --- | --- | --- |
| ~~Window-state persistence~~ done in-house (PR #100; the plugin hardcodes `app_config_dir` and would bypass `ROVE_DESKTOP_ROOT`) | window resets to 1400×900 every launch | hours |
| ~~App menu (Edit/View/Window + About)~~ done (PR #100) | Tauri default menu is bare | hours |
| Tray icon / minimize-to-tray (tauri core `tray` module, `tray-icon` feature) | long-running agent should not die on window close | 1–2 d |
| Notifications (`tauri-plugin-notification`) | surface run-done / needs_attention when unfocused | 1–2 d |
| Drag-drop workspace folders onto window | nicer than picker only | hours |
| Better packaging metadata | `copyright: ""`, generic description, `DeveloperTool` category | minutes |
| Auto-update (`tauri-plugin-updater`) | needs signed release channel first | later |
| Deep links `rove://` | nice-to-have | later |

### Needs a real OS to verify (acceptance, not code)

| Item | Where |
| --- | --- |
| Full installed-app flow (install → onboard → workspace → run → restart) | macOS arm64 first (owner's machine) |
| Folder selection on Windows/Linux | Windows/Linux boxes |
| MSI/NSIS install + uninstall, perMachine semantics | Windows |
| `.deb`/`.AppImage` install | Linux |
| Data-dir migration / second-instance refusal on each OS | all |

### Needs outside resources (blocked, owner)

| Item | Blocks on |
| --- | --- |
| macOS signing + notarization | Apple Developer cert ($99/yr) |
| Windows code signing | OV/EV cert |
| Clean-machine validation | spare/VM machines |
| Formal release channel + update server | hosting |

## Universality plan (Windows + macOS both first-class)

The architecture is already universal — one Rust workspace, one React UI,
one embedded API. What is not yet universal is **proof**. Three tracks:

### Track 1 — feature parity matrix (code)

Keep a single source of truth for "what the host can do per OS" so a new
feature lands everywhere or is explicitly platform-gated:

| Capability | Windows | macOS | Linux |
| --- | --- | --- | --- |
| Native credential prompt | `CredUI` | loopback form (verified) | loopback form |
| Folder picker | rfd (untested) | rfd (untested) | rfd (untested) |
| Reveal in file manager | `explorer /select` | `open -R` | `xdg-open` |
| External link | `open` crate | `open` crate | `open` crate |
| Tray | Show/Quit menu, template icon | Show/Quit menu, template icon | Show/Quit menu |
| Notifications | not built | not built | not built |

Rule: when adding a host command, implement all three OS arms or document
the gap in `docs/user-guide.md` the same day.

### Track 2 — CI build matrix (proof)

Extend `.github/workflows/desktop.yml` into a packaging matrix so every OS
produces a build artifact on every relevant change — unsigned but proves
the toolchain:

```yaml
strategy:
  matrix:
    os: [ubuntu-latest, windows-latest, macos-latest]
steps:
  - pnpm install + pnpm build:desktop  (apps/web → desktop-dist)
  - cargo tauri build                  (native bundle per OS)
  - upload artifact
```

macOS runner gives `.app`/`.dmg`, Windows gives `.msi`/`.nsis`, Ubuntu
gives `.deb`/`.AppImage`. This replaces "we think it compiles" with "the
bundle exists" — unsigned is fine for internal validation.

### Track 3 — acceptance matrix (evidence)

Mirror the pilot matrix shape but per-OS:

| Scenario | macOS | Windows | Linux |
| --- | --- | --- | --- |
| Install → first launch → onboard provider | ✅ | ◻ | ◻ |
| Workspace pick + trust + run a turn | ✅ | ◻ | ◻ |
| Approve/deny tool call | ✅ | ◻ | ◻ |
| Close window mid-run → reopen → state intact | ✅ | ◻ | ◻ |
| Second instance refused | ✅ | ◻ | ◻ |
| Uninstall → data dir preserved | ✅ | ◻ | ◻ |

Each cell gets `evidence/desktop/<date>-<os>-<task>/PILOT.md` in the same
pre-registered format as the web pilot. Unsigned builds are acceptable for
the matrix; signing is a separate gate.

macOS column: **PASS 2026-11-14** — `evidence/desktop/2026-11-14-macos-acceptance/PILOT.md`.
Adhoc-signed `.app` from `~/Applications`, isolated `ROVE_DESKTOP_ROOT`,
live DeepSeek `deepseek-flash`. All six cells green plus the
stranded-queue Recover path. One open finding on `approved_policy`
recording (see PILOT.md), filed as a follow-up, not blocking.

## Suggested order for the next session

1. ~~**Packaging metadata cleanup**~~ done (PR #97).
2. ~~**CI build matrix**~~ done (PR #98) — ubuntu/windows/macos
   `cargo tauri build` produce unsigned bundles as workflow artifacts.
3. ~~**macOS acceptance run**~~ done (2026-11-14) — `.app` PASS on all six
   cells; evidence in `evidence/desktop/`.
4. ~~**Window-state + app menu**~~ done (PR #100): in-house tracker under
   `config/` + platform menu bar.
5. **Tray + notifications** (normal PRs, split): tray close-action first
   (this change), then notifications — biggest UX gap for an agent that
   runs long tasks.
6. **Windows/Linux acceptance** when a machine is available; signing/notary
   last, gated on certs.

## Non-goals (recorded to prevent re-scoping)

- No second UI for Desktop. Every Web console feature is the Desktop
  feature; host work is lifecycle/packaging/native integration only.
- No Electron, GPUI, or native-UI rewrite — Tauri 2 is the decided host
  (see `docs/decisions.md` history).
- No auto-updater before a signed release channel exists.
