# Rove Desktop Application

Native desktop application for Rove, built with Tauri 2 (currently locked to
2.11.5).

## Architecture

The Desktop application is a thin native shell that:
- Embeds the `rove-api` server on a random localhost port
- Loads the static Web UI (`apps/web`) in a WebView
- Provides platform-specific native commands (folder picker, file manager integration)
- Manages bearer token authentication between WebView and API server

**Key principle**: Desktop contains no business logic or state authority. All
application logic lives in `rove-runtime` and `rove-api`. Desktop is purely a
delivery vehicle.

## Components

### 1. API Server Lifecycle (`src/api_server.rs`)

- Binds to `127.0.0.1:0` and retains the listener to avoid a port race
- Starts the full `rove-api` server with ProductStore, routing, and middleware
- Polls authenticated `/product/runtime` until ready
- Provides graceful shutdown with a 5-second timeout followed by task abort

### 2. Configuration (`src/config.rs`)

- Generates a cryptographically random 32-byte bearer token on first launch
- Stores config in platform-specific directories:
  - **Windows**: `%APPDATA%\Rove\config\desktop.json`
  - **macOS**: `~/Library/Application Support/Rove/config/desktop.json`
  - **Linux**: `~/.config/rove/desktop.json` (XDG compliant)
- Optional `ROVE_DESKTOP_ROOT` must be absolute and places token config, state and logs under `config/`, `state/`, `logs/`. This does not move the shared provider catalog (`ROVE_CONFIG_ROOT`), project trust store (`ROVE_PROJECT_TRUST_STORE`), OS credential store or WebView preferences. Set the first two separately for an isolated trial.
- File permissions set to `0600` on Unix platforms

### 3. Tauri Commands (`src/commands.rs`)

Security-hardened commands exposed to the WebView:

- `workspace_select()`: Native folder picker dialog, wired into both shared-Web
  workspace forms through `@tauri-apps/api`
- `provider_credential_prompt()` (Windows): Opens the Windows credential UI, writes the
  API key directly to the OS keyring, and returns only a bounded receipt
- `get_app_paths()`: Returns config/state/logs directory paths
- `open_external(url)`: Opens URLs in default browser (http/https only)
- `show_in_folder(path)`: Shows file in native file manager (with path validation)

On macOS/Linux, the shared provider form sends its password input through the
existing authenticated loopback onboarding endpoint. The host advertises this
capability before page scripts run. The field is cleared immediately on submit;
keys never enter React state, local storage or responses. The same onboarding
service owns provider verification, keyring storage and failed-write compensation.

### 4. WebView Integration (`src/lib.rs`)

- Injects bearer token and API URL through a document-start, origin-bounded
  initialization script before page scripts run
- Configures CSP for the loopback API and WebSocket development transport
- Window size: 1400x900 (minimum 800x600)

## Development

### Prerequisites

- The repository Rust toolchain
- Node.js 22
- pnpm
- Tauri CLI through `pnpm dlx @tauri-apps/cli@2`

### Build

```bash
# Development build
cargo build -p rove-desktop

# Package with the shared static Web UI, from apps/desktop
pnpm dlx @tauri-apps/cli@2 build
```

### Run

```bash
# Run in development mode (hot-reload Web UI), from apps/desktop
pnpm dlx @tauri-apps/cli@2 dev

# Local macOS bundle smoke (uses the static UI, no dev server)
pnpm dlx @tauri-apps/cli@2 build --debug --bundles app --ci
# Run target/debug/bundle/macos/Rove.app after the build
```

### Test

```bash
# Unit tests
cargo test -p rove-desktop

# Integration tests
cargo test -p rove-desktop --test integration

# All tests
cargo test -p rove-desktop --all-targets
```

## Platform Support

### Windows

- Installer formats: `.msi` and NSIS `.exe`
- Install scope: per-machine under `Program Files`; this avoids the existing
  `%LOCALAPPDATA%\rove` user-state root
- Start menu shortcut: `Rove\Rove`
- Paths use backslashes (`\`)
- Config: `%APPDATA%\Rove`
- State: `%APPDATA%\Rove\state`
- Logs: `%APPDATA%\Rove\logs`

Build command from `apps/desktop`:
```bash
pnpm dlx @tauri-apps/cli@2 build --bundles "msi,nsis" --ci
```

The Tauri hooks set their working directory explicitly to `../web` relative to
`apps/desktop`. This lets Corepack select the version pinned by the Web package
before running `pnpm dev` or `pnpm build:desktop`. Bundle targets default to the
native formats for the current platform. Generated installers remain under
`target/release/bundle/` and are not committed.

### macOS

- Bundle format: `.app` / `.dmg`
- Config: `~/Library/Application Support/Rove/config`
- State: `~/Library/Application Support/Rove/state`
- Logs: `~/Library/Logs/Rove`

Build commands:
```bash
# Intel
pnpm dlx @tauri-apps/cli@2 build --target x86_64-apple-darwin

# Apple Silicon
pnpm dlx @tauri-apps/cli@2 build --target aarch64-apple-darwin
```

A local Apple Silicon debug and release `.app` has been checked with native folder selection,
offline task completion, normal close, restart and continued conversation. Clean
machine installation, release signing/notarization and credentialed tasks are
still unverified. Windows uses its native credential window; macOS/Linux use **Verify and save**
in the shared provider form. An OS keyring service must be available (on Linux
this usually requires a logged-in desktop session).

Code signing and notarization remain manual. Configure the Apple signing
credentials for distribution; a local debug bundle does not establish release readiness.

### Linux

- Package formats: `.deb`, `.AppImage`, `.rpm`
- Config: `$XDG_CONFIG_HOME/rove` (fallback: `~/.config/rove`)
- State: `$XDG_DATA_HOME/rove` (fallback: `~/.local/share/rove`)
- Logs: `$XDG_STATE_HOME/rove/logs` (fallback: `~/.local/state/rove/logs`)

Ubuntu/Debian builds require the native GTK/WebKit development packages used
by Tauri:

```bash
sudo apt-get update
sudo apt-get install --no-install-recommends -y \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  libwebkit2gtk-4.1-dev
```

Build command:
```bash
pnpm dlx @tauri-apps/cli@2 build --target x86_64-unknown-linux-gnu
```

## Security

### Authentication

- Bearer token generated with `rand::thread_rng()` (32 bytes)
- Encoded as URL-safe base64 without padding
- Stored in `desktop.json` with file permissions `0600` (Unix)
- Injected through document-start initialization before page scripts run
- Never logged or exposed to external services

### Input Validation

- **URLs**: Only `http://` and `https://` schemes allowed in `open_external()`
- **Paths**: Must be absolute, must exist, no `..` traversal in `show_in_folder()`
- **Workspace selection**: Validated against filesystem before returning to WebView
- **Provider credentials**: Profile ids use the shared catalog character set;
  labels reject control characters; receipts contain only keyring metadata.
  The in-memory secret is zeroized after the keyring write.

### Content Security Policy

```json
{
  "default-src": "'self'",
  "connect-src": "'self' http://localhost:* ws://localhost:* wss://localhost:*",
  "style-src": "'self' 'unsafe-inline'",
  "script-src": "'self' 'unsafe-inline' 'unsafe-eval'",
  "img-src": "'self' data: blob:",
  "font-src": "'self' data:"
}
```

## Known Limitations (D0)

- **No auto-update**: Manual download and install required
- **No remote crash reporting**: A payload-redacted panic marker is written to
  the platform logs directory
- **No sandboxing**: Full filesystem and network access (mitigated by command allowlist)
- **Manual signing**: Code signing and notarization must be done manually on macOS
- **Single instance**: No enforcement; multiple instances can run simultaneously

These limitations are acceptable for D0 (internal dogfooding) and will be addressed in post-D0 releases.

## Troubleshooting

### Desktop won't launch

1. Check `%APPDATA%\Rove\logs`, `~/Library/Logs/Rove`, or
   `$XDG_STATE_HOME/rove/logs` for the current platform
2. Verify API server can start: `cargo run -p rove-api`
3. Verify Web UI builds: `cd apps/web && pnpm build`

### WebView shows blank page

1. Open DevTools (F12 in debug builds)
2. Check console for JavaScript errors
3. Verify `Boolean(window.__ROVE_TOKEN__)` and `Boolean(window.__ROVE_API_URL__)` are true; never print or copy the token
4. Check Network tab for API call failures

### API calls fail with 401 Unauthorized

1. Verify injection without revealing the secret: `Boolean(window.__ROVE_TOKEN__)`
2. Check that Web UI is sending `Authorization: Bearer <token>` header
3. Close the app and restart it using the same config root; do not paste token values into logs or issue reports

### Port conflict

Desktop uses `localhost:0` to find an available port automatically. If no ports are available (unlikely), the application will fail to start with an error message.

## Architecture Decision Records

### Why embed the API server instead of spawning a child process?

**Decision**: Embed `rove-api` as a library and start it in a Tokio task.

**Rationale**:
- Simpler lifecycle management (no process spawning, no orphaned processes)
- Shared memory and zero-copy data structures
- Easier to test (no IPC or socket mocking required)
- Graceful shutdown guaranteed (no SIGTERM / SIGKILL coordination)

**Trade-off**: Crash in API server takes down the entire Desktop app. Acceptable for D0.

### Why inject bearer token into WebView instead of using Tauri's state?

**Decision**: Inject token via a document-start `window.__ROVE_TOKEN__` global
after the embedded API is ready and before the static page scripts run.

**Rationale**:
- Web UI can use the same authentication logic as browser deployment
- No Tauri-specific API calls required in Web UI
- Token available synchronously (no async state fetch required)
- Web UI remains Tauri-agnostic (can be deployed standalone)

### Why random port allocation instead of fixed port?

**Decision**: Bind to `localhost:0` to find an available port.

**Rationale**:
- Avoids port conflicts with other services
- Allows multiple Rove instances to run simultaneously (if needed)
- No user configuration required

**Trade-off**: Port changes on each launch. Acceptable because WebView is controlled and receives the URL automatically.

## Licensing

The desktop crate and repository are licensed under the root MIT `LICENSE`.
