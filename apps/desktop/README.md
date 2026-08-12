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
- File permissions set to `0600` on Unix platforms

### 3. Tauri Commands (`src/commands.rs`)

Security-hardened commands exposed to the WebView:

- `workspace_select()`: Native folder picker dialog, wired into both shared-Web
  workspace forms through `@tauri-apps/api`
- `get_app_paths()`: Returns config/state/logs directory paths
- `open_external(url)`: Opens URLs in default browser (http/https only)
- `show_in_folder(path)`: Shows file in native file manager (with path validation)

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

# Release build
cargo build --release -p rove-desktop
```

### Run

```bash
# Run in development mode (hot-reload Web UI), from apps/desktop
pnpm dlx @tauri-apps/cli@2 dev

# Run release binary
cargo run --release -p rove-desktop
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

- Installer format: `.msi`
- Paths use backslashes (`\`)
- Config: `%APPDATA%\Rove`
- State: `%APPDATA%\Rove\state`
- Logs: `%APPDATA%\Rove\logs`

Build command from `apps/desktop`:
```bash
pnpm dlx @tauri-apps/cli@2 build --target x86_64-pc-windows-msvc
```

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

**Note**: Code signing and notarization are manual for D0. Set `APPLE_SIGNING_IDENTITY` and `APPLE_ID` environment variables, or disable signing in `tauri.conf.json` for local testing.

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
3. Verify `window.__ROVE_TOKEN__` and `window.__ROVE_API_URL__` are set
4. Check Network tab for API call failures

### API calls fail with 401 Unauthorized

1. Verify bearer token is injected: Open DevTools Console, type `window.__ROVE_TOKEN__`
2. Check that Web UI is sending `Authorization: Bearer <token>` header
3. Verify the token matches the platform-specific `desktop.json` path listed
   under [Configuration](#2-configuration-srcconfigrs)

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

The desktop crate currently declares MIT package metadata. The repository does
not yet contain a root license file; add and review one before making a public
distribution claim.
