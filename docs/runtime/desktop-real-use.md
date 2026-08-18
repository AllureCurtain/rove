# Windows Desktop Real-Use Status

> Status: **Current / Partially Implemented**
>
> Updated: 2026-08-18
>
> Scope: Desktop-owned D1-D5 work that can be completed before the shared
> Provider onboarding contract lands. D6 and the final A gate are not complete.

## Current Contract

Desktop remains a thin Tauri host over the shared `rove-api`, Runtime,
ProductStore, Tool Registry, provider catalog, canonical events, and durable
run state. It does not own an Agent loop or a private event protocol.

The current Desktop slice provides:

- Windows `provider_credential_prompt`, which accepts only non-secret profile
  metadata, opens the native credential UI, writes a unique Rove-owned OS
  keyring entry, zeroizes the in-memory secret, and returns only a keyring
  receipt;
- a typed WebView wrapper that rejects browser use, malformed metadata, and
  malformed receipts without accepting an API key argument;
- the existing native folder picker and exact Product Workspace/Session path;
- bearer-authenticated Desktop SSE with canonical event ids, focused-job
  attachment, and reconnect from the last observed event id;
- canonical Chat/Inspector projection for tool calls, files, artifacts, usage,
  approvals, inputs, cancellation, diff, and terminal state;
- payload-free startup failure logging plus a native error dialog with the log
  location;
- explicit Windows MSI and NSIS packages. NSIS uses `perMachine` installation
  under `Program Files` and creates `Rove\Rove` in the Start menu. It must not
  install into `%LOCALAPPDATA%\Rove`, which is an existing Rove user-state root.

Raw provider keys are not fields in React state, localStorage, ProductStore,
ordinary Product API requests, Desktop JSON config, trace, report, or the
credential command receipt.

## Verified On This Branch

The following checks passed on Windows from a worktree with no pre-existing
release executable or bundle:

```powershell
cargo test -p rove-desktop --all-targets
cd apps/web
pnpm exec vitest run platform/desktop-commands.test.ts lib/rove-client.test.ts
pnpm typecheck
cd ../desktop
pnpm dlx @tauri-apps/cli@2 build --bundles "msi,nsis" --ci
```

The bundler produced both generated, untracked packages under
`target/release/bundle/`. The build verified that Tauri runs its Web hook from
`apps/`, so `pnpm --dir web build:desktop` is the correct checked-in command.

The generated WiX and NSIS sources confirm per-machine installation, a
`Program Files\Rove` default, and a Start menu shortcut. Actual installation
was not run in the non-administrator implementation session; build success is
not installation evidence.

## Shared Dependency

At the time of this record, `origin/main` and
`origin/feature/tui-real-use-final` both point to `9611926`. They do not yet
contain the required shared `ProviderOnboardingService`, Catalog CAS/probe
transaction, or the Product API request needed to publish a newly created
keyring receipt.

The host prompt and Web wrapper therefore stop at the safe receipt boundary.
Desktop must not write the Provider Catalog directly or invent a Desktop-only
profile field. After the shared Provider commit lands, Desktop must merge it,
call that service through the Product API, compensate the unique keyring entry
on probe/CAS failure, and expose the complete create/test/use flow in Settings.

## Installed Journey

Run this only after shared Provider gate F4 and TUI gate T7 pass and Desktop has
merged their shared contract:

1. Build MSI and NSIS packages with the command above and record their SHA-256
   hashes outside the repository.
2. Install one package with administrator approval and launch `Rove` from the
   Start menu. Do not set a temporary PowerShell provider variable.
3. Configure or select `siliconflow-deepseek-v3-2`, using provider type
   `openai`, base URL `https://api.siliconflow.cn/v1`, and model
   `deepseek-ai/DeepSeek-V3.2`. Confirm Settings displays only safe metadata.
4. Select the fixed, secret-free demo Git repository with the native folder
   picker and create a Product Session.
5. Ask `当前目录有哪些主要内容？请先实际检查，再给出简短说明。`, then
   `这个程序的入口在哪里？请读取相关文件并说明依据。`.
6. Expand Inspector and verify canonical list/search/read tool calls, bounded
   arguments/results, Files, Artifacts, Usage, and grounded final answers.
7. Run the specified small README modification, approve it, and verify the
   canonical diff and the minimal related test result.
8. Interrupt SSE without sending another turn; verify focused reconnect does
   not duplicate the task.
9. Close Desktop, launch it again from the Start menu, and verify the exact
   Workspace, Session, transcript, tool evidence, and terminal state restore.
10. Uninstall and confirm user Provider Catalog, keyring entries, workspaces,
    ProductStore state, and runtime evidence are retained.

Evidence must remain outside the repository, include real command exit codes,
and be labeled `credentialed external`. Fake or mocked tests cannot satisfy any
step in this installed journey.

## Open Gates

- Shared keyring onboarding service and Product API publication: waiting for
  the TUI-owned shared contract.
- Native prompt plus create/test/use Settings flow: not integrated until that
  contract lands.
- SiliconFlow inventory, streaming, native tool-call history, and two-turn
  Desktop run: not run.
- Installed Start menu journey, restart restoration, and uninstall retention:
  not run in the current non-administrator session.
- D6 and final A gate: not met; the final implementation plan must remain
  `Not Implemented`.
