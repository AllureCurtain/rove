# Windows Desktop Real-Use Status

> Status: **Current / D1-D6 Complete on Windows**
>
> Updated: 2026-08-26
>
> Scope: Desktop-owned D1-D5 work integrated with the shared Provider onboarding
> contract, plus the D6 installed-state gate passed against the real SiliconFlow
> provider. The final section 10.1 A gate is still outstanding: it must run on
> the final `main` after both branches merge, and this Windows-only evidence
> does not substitute for it.

## Current Contract

Desktop remains a thin Tauri host over the shared `rove-api`, Runtime,
ProductStore, Tool Registry, provider catalog, canonical events, and durable
run state. It does not own an Agent loop or a private event protocol.

The current Desktop slice provides:

- Windows `provider_credential_prompt`, which accepts only non-secret profile
  metadata, opens the native credential UI with `CREDUI_FLAGS_DO_NOT_PERSIST`,
  passes the raw credential directly to the shared
  `ProviderOnboardingService` through an in-process `ApiState` facade, and
  zeroizes the in-memory secret on every path;
- `provider_profile_probe` and `provider_profile_use`, which re-probe real
  Provider inventory and persist the shared Catalog default selection through
  the same CAS path used by the CLI/TUI, without serializing a credential;
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
credential command receipt. There is no HTTP route that accepts a provider
secret; onboarding is in-process only and the credential is a separate
non-serializable argument.

Settings splits by host: the Desktop host renders native credential
onboarding, probe, Catalog publication, refresh, and selection with a built-in
SiliconFlow preset, while the browser keeps the existing env/file/reference
CRUD and never receives a secret path.

## Verified On This Branch

The following checks passed on Windows from a worktree with no pre-existing
release executable or bundle:

```powershell
cargo fmt --all --check                     # exit 0
cargo clippy --workspace --all-targets -- -D warnings   # exit 0
cargo test --workspace                      # exit 0, 1567 passed / 0 failed
cargo test -p rove-api                      # 137 passed
cargo test -p rove-desktop --all-targets    # 13 lib + 3 integration passed
cd apps/web
pnpm test                                   # 37 files / 255 tests passed
pnpm typecheck                              # exit 0
pnpm build:desktop                          # exit 0
cd ../desktop
pnpm dlx @tauri-apps/cli@2 build --bundles "msi,nsis" --ci
```

The deterministic A1 code gate passes on this branch. `pnpm test:e2e`
(Playwright) was not run here and remains part of the final A1 gate on `main`.

The bundler produced both generated, untracked packages. `CARGO_TARGET_DIR` was
redirected to `C:\rove-build\desktop-real-use-final` because the D: volume was
short on space, so the artifacts landed under
`C:\rove-build\desktop-real-use-final\release\bundle\{msi,nsis}\` rather than the
in-repo `target/release/bundle/`. With the default target dir the in-repo path
applies. The build verified that Tauri runs its Web hook from `apps/`, so
`pnpm --dir web build:desktop` is the correct checked-in command.

The generated WiX and NSIS sources declare per-machine installation, a
`Program Files\Rove` default, and a Start menu shortcut, and the installed-state
gate below confirms all three in practice.

## Installed-State Gate (D6)

D6 passed on 2026-08-26 against the real SiliconFlow provider (`openai`,
`https://api.siliconflow.cn/v1`, `deepseek-ai/DeepSeek-V3.2`), with the key read
only from the Windows keyring and the Desktop launched from the Start menu
shortcut. No Fake provider was involved, and no temporary environment variable
was set. The evidence package is at `C:\rove-evidence\d6-desktop\`
(`manifest.json` indexes every artifact with a sha256 digest); it is generated
and deliberately not committed.

All nine items pass: install, uninstall/reinstall round trip, credential
resolution after Start-menu launch, two read-only turns on the real model,
Chat/Inspector showing real tool calls, a modification turn with approval plus
change plus test plus diff, SSE disconnect/reconnect without duplicate
submission, restart restoring workspace/session/terminal state, and reinstall
plus relaunch restoring state.

Notes worth carrying forward:

- Both NSIS packages are `perMachine`. Install and uninstall must run as an
  elevated child; driving `uninstall.exe` from an unelevated session fails with
  `WinError 740`.
- Uninstall preserves user data: `%APPDATA%\Rove` stayed at 146 files /
  11204219 bytes across installed, uninstalled, and reinstalled states.
- There is no `/health` route. `/product/workspaces` serves as the readiness
  probe and simultaneously proves the persisted bearer token is accepted.
- The embedded API binds a fresh random loopback port per launch and does not
  persist it; `%APPDATA%\Rove\config\desktop.json` holds only `bearer_token`.
  Resolve the port from the process's own listeners.
- `/jobs/{id}/state` returns 404 after a restart. The job registry is ephemeral
  by design; terminal state is recovered from the durable run report instead.
- Screenshots were not captured. Recorded as a gap rather than filled with a
  synthetic image.

Four product defects were confirmed while running the gate and are listed in the
manifest under `known_product_defects_found`. None were fixed here, because
section 6 forbids this branch from unilaterally changing Core Agent loop,
Provider wire protocol, or Tool Registry permission behaviour:

1. `needs_attention` is a terminal deadlock. A run that ends without a final
   answer strands the session with no recovery endpoint.
2. Project config is never loaded on the API/Desktop path
   (`project_config_loaded` is hardcoded false) even when the
   `project_configuration` trust capability is granted.
3. `max_model_turns_per_step = 4` is unreachable from Desktop.
4. The product UI cannot select an agent profile:
   `CreateProductMessageRequest` sets `deny_unknown_fields`, so no `agent` field
   can be sent even though `POST /jobs` honours one.

## Shared Dependency

Resolved. The shared secure onboarding contract landed in `cc9799f` and is
contained in `origin/main` at `8a4e141`, which this branch has merged.

Desktop now consumes that contract instead of stopping at a receipt boundary:

- `ApiState::onboard_product_provider`, `probe_product_provider`, and
  `use_product_provider` wrap the shared `ProviderOnboardingService`, so
  keyring storage, real inventory probing, Catalog CAS publication, and
  failure compensation stay owned by the shared service;
- the Desktop-private `com.rove.agent.provider` keyring receipt was removed;
  Desktop no longer writes the Provider Catalog or invents a profile field;
- a projection failure after a successful publication returns the typed
  `provider_product_projection` code and asks for reconciliation rather than
  silently diverging from the shared Catalog.

## Installed Journey

The shared contract is merged, so this journey is now unblocked on code. It
still requires an administrator/UAC-capable interactive Windows session, which
the implementation session did not have:

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

Closed on this branch:

- Shared keyring onboarding service and in-process publication: integrated.
- Native prompt plus create/test/use Settings flow: integrated and covered by
  deterministic host and Web tests.

- SiliconFlow inventory, streaming, native tool-call history, and the two-turn
  Desktop run against `deepseek-ai/DeepSeek-V3.2`: run and passing against the
  installed Desktop. Live inventory returned 95 models with the locked model
  present; 41 native tool calls all carried a provider `tool_use_id`.
- Installed Start menu journey, restart restoration, and uninstall retention:
  run and passing. Install and uninstall need an elevated child because the NSIS
  package is `perMachine`.
- D6: met on Windows. See the installed-state gate section above.

Still open, and blocking any "real-use complete" claim:

- Section 10.1 A gate: not met. It must run on the final `main` after both
  branches merge. The D6 evidence here is Windows-only and single-machine, so it
  does not substitute for that gate, for the A3 installed-state TUI gate, or for
  any claim about unverified platforms.
- `pnpm test:e2e` (Playwright) was not run on this branch and remains part of
  the final A1 gate on `main`.
- Desktop screenshots for the D6 journey: not captured.
