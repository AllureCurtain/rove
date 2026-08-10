# Rove Desktop

Desktop application for Rove, powered by Tauri 2.

## Architecture

Rove Desktop embeds the complete Rove API server (runtime, engine, ProductStore) into a native desktop application using Tauri 2. The architecture follows the **no-second-backend** principle:

- **API Server**: Full `rove-api` runtime embedded in the desktop process
- **WebView**: Reuses the same Web UI bundle as browser deployment
- **Process Topology**: Single process with embedded server and WebView
- **State Authority**: ProductStore remains the single source of truth

## Development

### Prerequisites

- Rust 1.70+
- Node.js 18+
- pnpm
- Tauri CLI: `cargo install tauri-cli@2.1`

### Build

```bash
# Build the desktop application
cd apps/desktop
cargo build

# Build with Tauri bundler
cargo tauri build
```

### Run

```bash
# Development mode (with Web dev server)
cargo tauri dev

# Production mode
cargo run --release
```

## Configuration

Desktop configuration is stored in platform-specific locations:

- **Windows**: `%APPDATA%\Rove\config\desktop.json`
- **macOS**: `~/Library/Application Support/Rove/config/desktop.json`
- **Linux**: `~/.config/rove/desktop.json` (or `$XDG_CONFIG_HOME/rove/desktop.json`)

### Bearer Token

A random bearer token is generated on first launch and persisted in `desktop.json` with file permissions `0600`. This token authenticates the WebView to the embedded API server.

## Platform Support

### Windows

- Target: `x86_64-pc-windows-msvc`
- Installer: `.msi`
- Paths: `%APPDATA%\Rove\`

```bash
cargo tauri build --target x86_64-pc-windows-msvc
```

### macOS

- Targets: `x86_64-apple-darwin`, `aarch64-apple-darwin`
- Bundle: `.app` / `.dmg`
- Paths: `~/Library/Application Support/Rove/`

```bash
# Intel
cargo tauri build --target x86_64-apple-darwin

# Apple Silicon
cargo tauri build --target aarch64-apple-darwin
```

**Note**: Code signing and notarization are manual steps for D0. See [Tauri documentation](https://tauri.app/v1/guides/distribution/sign-macos/) for details.

### Linux

- Target: `x86_64-unknown-linux-gnu`
- Packages: `.deb`, `.AppImage`
- Paths: `~/.config/rove/` (config), `~/.local/share/rove/` (state)

```bash
cargo tauri build --target x86_64-unknown-linux-gnu
```

## Security

### IPC Commands

Only the following Tauri commands are exposed to the WebView:

- `get_app_paths()` - Returns config/state/logs directories
- `workspace_select()` - Opens native folder picker
- `open_external(url)` - Opens URL in default browser (http/https only)
- `show_in_folder(path)` - Opens file manager at path

All commands include input validation:
- URLs: Only `http://` and `https://` schemes allowed
- Paths: Must be absolute, no traversal patterns, must exist

### Content Security Policy

```json
{
  "default-src": "'self'",
  "connect-src": "'self' http://localhost:* ws://localhost:*",
  "style-src": "'self' 'unsafe-inline'",
  "script-src": "'self' 'unsafe-inline' 'unsafe-eval'",
  "img-src": "'self' data: blob:",
  "font-src": "'self' data:"
}
```

### Bearer Token Storage

- Token stored with file permissions `0600` (Unix)
- Token never logged or exposed to external services
- Token regenerated if `desktop.json` is deleted

## Testing

```bash
# Run all tests
cargo test -p rove-desktop

# Run with logs
RUST_LOG=debug cargo test -p rove-desktop -- --nocapture
```

## Known Limitations (D0)

- **No auto-update**: Manual download and install required
- **No system tray**: Application runs as a normal window
- **No code signing**: macOS will show "unidentified developer" warning
- **No notarization**: macOS Gatekeeper bypass required (`xattr -d com.apple.quarantine`)
- **Manual packaging**: Distribution via manual download, not app stores

These features are deferred to post-D0 releases.

## Troubleshooting

### "Command failed: cargo tauri"

Ensure Tauri CLI is installed:
```bash
cargo install tauri-cli@2.1
```

### API server fails to start

Check logs at:
- Windows: `%APPDATA%\Rove\logs\`
- macOS: `~/Library/Logs/Rove/`
- Linux: `~/.local/state/rove/logs/`

### WebView shows blank page

1. Check that `apps/web` builds successfully: `cd apps/web && pnpm build`
2. Verify `tauri.conf.json` `frontendDist` points to `../web/dist`
3. Check browser console in dev tools (Ctrl+Shift+I / Cmd+Option+I)

## License

MIT
