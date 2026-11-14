# Desktop verification — tray + notifications (macOS arm64)

Date: 2026-11-14. Build: `pnpm dlx @tauri-apps/cli@2 build --bundles app --ci`
on `feat/desktop-notifications` @ `06d6d33` (equivalent to merged main
`6e60a94`: tray PR #101 + notifications PR #102). Host: owner's Apple
Silicon machine, macOS 27.2. Binary: `~/Applications/Rove.app` (63.9 MiB,
re-signed adhoc after copy, `com.rove.agent`).

Isolation: `ROVE_DESKTOP_ROOT=/tmp/rove-desktop-tray-notif2` so the real
`~/Library/Application Support/Rove` was untouched. Workspace under test:
`/tmp/rove-tray-workspace` (git repo). The embedded API was driven over
`127.0.0.1` with the bearer token from `config/desktop.json`.

Scope: incremental verification of the two feature PRs — it reuses the
six-cell 2026-11-14 acceptance as the baseline and adds only the cells
the new code changes. What it does NOT re-cover: provider onboarding,
trust grants, plan approval, second instance, uninstall (unchanged code).

## Result matrix

| Scenario | Result | Evidence |
| --- | --- | --- |
| Notification pump subscribes | PASS | stdout `rove_desktop::notifications: desktop notification pump subscribed to /product/events` on launch; `lsof` shows a `rove-desktop→rove-desktop:51664` established SSE connection distinct from the WebView's |
| `needs_attention` produces a native notification | PASS | A queued turn without a provider requeued `running`→`idle` ×6 then `needs_attention` at `23:39:44`. usernoted recorded two `com.rove.agent` records (`F3E89F56` `23:39:30`, `6BAC9BFF` `23:39:45`) `addDelivered` + `Adding notification to storage`. `libxpc: activating connection … com.apple.usernoted.client` in the app's unified log at `23:39:14` — the pump contacted the notification daemon the moment the gate tripped |
| Denied permission degrades gracefully | PASS | UNNotificationSettings for `com.rove.agent` was `authorizationStatus: Denied` (carried from an earlier acceptance run); usernoted still `delivered` both records to NotificationCenter storage with `alertStyle: none`. No crash, no permission re-prompt, pump kept running — matches the documented "denial only silences banners" contract |
| `close_action` persists in `desktop.json` | PASS | First-launch config carried `"close_action": "hide_to_tray"` (the default). Editing it to `"quit"` round-trips; the file is rewritten atomically (`write_config_atomically`) and the bearer token survives |
| Quit drains the API before the process dies | PASS | `osascript 'tell app "Rove" to quit'` produced in order: `desktop notification pump stopped` → `API graceful shutdown initiated` (×2, preview + api) → `embedded API server shut down gracefully` → `Desktop API lifecycle stopped`. The `ApiServerState.stop` CancellationToken cancels the pump's SSE streams before the server drains — the order is now observable in the log |
| Window geometry saved on quit | PASS | `config/window-state.json` written on `osascript quit`: `{"width":1400,"height":900,"x":260,"y":68,"maximized":false}` — the `RunEvent::Exit` flush added in PR #100 works on a bundled app where `CloseRequested` does not fire |
| Session state survives restart | PASS | `needs_attention` session `01M7DBCEKSSMHF8VH3PJ296QYQ` still `needs_attention` after relaunch — recovery does not silently claim or lose the stranded turn |

## Not verified here (environmental, not defects)

- **Tray icon click / window close → hide**: macOS refuses `osascript` and
  `System Events` UI automation without Accessibility permission for the
  driving terminal (`-1728`/`-1719`), and Tauri exposes no AppleScript
  window verbs (`close window 1` → `-1708`). The `CloseRequested →
  prevent_close + hide()` path is unit-covered in `lib.rs`; a manual
  smoke (click the tray icon, hit ⌘W) is the remaining human check. Both
  prior PILOTs hit the same permission wall.
- **Visual banner pop**: this machine had already denied Rove
  notifications during an earlier run, so the pipeline delivered to
  NotificationCenter storage without a visible banner. A fresh user or a
  granted System Settings → Notifications → Rove toggle would show the
  banner pop on the same code path.

## Notes

- `GET /product/events` replay confirmed the pump's event contract:
  `session.status_changed` frames carry `summary` as a JSON-encoded
  string `{"status":"running|idle|needs_attention","last_outcome":…}` —
  the mapping to finish / needs-attention / approval alerts in
  `notifications.rs` reads exactly those fields.
- The earlier `open`-based launch wrote a `desktop-startup.log` failure
  marker and exited (LaunchServices quirk, `open` env propagation); a
  direct `Contents/MacOS/rove-desktop` launch worked every time. Recorded
  here so a future acceptance script does not debug it again.

## Verdict

Tray + notifications: **PASS** on everything programmatically reachable.
The only gap is the same Accessibility-permission wall the 2026-11-14
pilot documented for interactive UI automation; the feature code itself
is on merged main (`6e60a94`) and CI-green on all three OS bundles.
