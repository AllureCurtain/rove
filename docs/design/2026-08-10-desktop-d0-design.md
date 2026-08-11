# Desktop D0: Tauri 2 Integration Design

> Status: **Implemented; Windows packaging verified, macOS/Linux evidence pending**
>
> Date: 2026-08-10
>
> Baseline: `program/full-delivery` after CP7 (commit 0c2d18c)
>
> Target: Checkpoint 8 of Post-Coding-Tool V2 Full Delivery

## 1. Mission

Deliver a native Desktop application that wraps the existing Web UI with Tauri 2, 
reusing the complete Runtime/API/Engine/ProductStore infrastructure without 
creating a second backend architecture.

### Success Criteria

- Users can install and run rove as a native Desktop app on Windows/macOS/Linux
- Desktop uses the exact same Web UI bundle as the Web product
- Desktop reuses Runtime, API, Engine, ProductStore, event streams, and tool 
  registries without duplication
- No Desktop-only backend logic, state authority, or execution loop
- All security boundaries (filesystem, process, network) remain enforced
- Clean packaging, startup/shutdown, crash recovery, and platform integration

### Non-Goals (Deferred)

- Auto-update mechanism (manual installation for D0)
- System tray integration
- Multi-window support
- Custom protocol handlers (`rove://`)
- Native notifications
- Sparkle/WinSparkle integration
- Code signing and notarization (documented as manual for D0)

---

## 2. Architecture

### Process Topology

```text
┌─────────────────────────────────────────────┐
│  Tauri Main Process (Rust)                  │
│  ┌───────────────────────────────────────┐  │
│  │ Runtime API Server (rove-api)         │  │
│  │ - ProductStore                        │  │
│  │ - Engine                              │  │
│  │ - Tool Registry                       │  │
│  │ - MCP Proxy                           │  │
│  │ - Session Management                  │  │
│  └───────────────────────────────────────┘  │
│  ┌───────────────────────────────────────┐  │
│  │ Tauri Commands (allowlisted IPC)     │  │
│  │ - workspace_select                    │  │
│  │ - get_app_paths                       │  │
│  │ - open_external                       │  │
│  │ - show_in_folder                      │  │
│  └───────────────────────────────────────┘  │
└─────────────────────────────────────────────┘
         ↕ (authenticated loopback HTTP + SSE)
┌─────────────────────────────────────────────┐
│  Tauri WebView (apps/web bundle)           │
│  - Product UI (React)                       │
│  - Same code as Web deployment              │
│  - Calls API via http://localhost:PORT      │
└─────────────────────────────────────────────┘
```

### Key Decisions

1. **Embedded API Server**: The Tauri main process starts `rove-api` on a random 
   localhost port, just like `rove-cli serve`.

2. **Shared UI Bundle**: Desktop loads the exact `apps/web` static build output. No
   Desktop-specific UI fork.

3. **No Second Backend**: Desktop does NOT create its own Engine, planner, tool 
   executor, or state store. It delegates everything to the Runtime API.

4. **IPC Allowlist**: Only expose minimal, bounded Tauri commands. Never leak 
   raw API keys, filesystem traversal, or broad process authority.

5. **ProductStore Ownership**: The embedded API server owns the ProductStore 
   (`.rove/` state). Desktop does not bypass it.

---

## 3. Tauri 2 Configuration

### Cargo Workspace Structure

```text
apps/
├── desktop/          # New Tauri application
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── src/
│   │   ├── main.rs          # Tauri main process entry
│   │   ├── api_server.rs    # Embedded rove-api lifecycle
│   │   ├── commands.rs      # Allowlisted Tauri commands
│   │   └── lib.rs
│   ├── icons/               # Application icons
│   └── build.rs
```

### Dependencies

```toml
[dependencies]
tauri = { version = "2.1", features = [] }
rove-api = { path = "../api" }
tokio = { version = "1", features = ["full"] }
tokio-util = "0.7"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
anyhow = "1"
```

### tauri.conf.json

```json
{
  "productName": "Rove",
  "version": "0.1.0",
  "identifier": "com.rove.agent",
  "build": {
    "beforeDevCommand": "pnpm --dir web dev",
    "beforeBuildCommand": "pnpm --dir web build:desktop",
    "devUrl": "http://localhost:3000",
    "frontendDist": "../web/desktop-dist"
  },
  "app": {
    "windows": [
      {
        "create": false,
        "title": "Rove",
        "width": 1400,
        "height": 900,
        "minWidth": 800,
        "minHeight": 600,
        "resizable": true,
        "fullscreen": false
      }
    ],
    "security": {
      "csp": {
        "default-src": "'self'",
        "connect-src": "'self' http://localhost:* http://127.0.0.1:* ws://localhost:* ws://127.0.0.1:*",
        "style-src": "'self' 'unsafe-inline'",
        "script-src": "'self' 'unsafe-inline' 'unsafe-eval'",
        "img-src": "'self' data: blob:",
        "font-src": "'self' data:"
      }
    }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": ["icons/32x32.png", "icons/128x128.png",
      "icons/128x128@2x.png", "icons/icon.icns", "icons/icon.ico"]
  }
}
```

---

## 4. API Server Lifecycle

### Startup Sequence

1. **Parse Configuration**: Load bearer token and platform config/state/log roots
2. **Find Available Port**: Bind to `127.0.0.1:0`, retaining the listener
3. **Start API Server**: Launch the shared `rove-api` router/state in-process
4. **Wait for Readiness**: Poll authenticated `/product/runtime` until ready
5. **Launch WebView**: Build the configured window only after API readiness
6. **Inject Token**: Use a document-start, origin-bounded runtime global; do not
   persist the token in Web localStorage

### Shutdown Sequence

1. **Window Close**: User closes the window or quits the app
2. **Cancel API Server**: Send cancellation signal to API server task
3. **Graceful Drain**: Wait up to 5 seconds for in-flight requests to complete
4. **Force Abort**: If timeout expires, abort and await the server task
5. **Clean Temp State**: No temporary runtime state is created outside the
   platform state directory

### Crash Recovery

- If API server panics or exits unexpectedly, show error dialog and exit
- Do NOT auto-restart the server in a loop (prevents runaway failures)
- Write a payload-redacted panic marker to the platform logs directory; do not
  copy panic payloads into logs

---

## 5. Allowlisted Tauri Commands

### `workspace_select`

```rust
#[tauri::command]
async fn workspace_select() -> Result<Option<String>, String> {
    // Open native folder picker dialog
    // Return selected path or None if cancelled
    // Validate path is absolute and exists
}
```

**Security**: Only returns user-selected paths. No arbitrary path injection.
The shared Web product calls this command only when the injected Desktop
transport is present; browser deployments retain their absolute-path form.

### `get_app_paths`

```rust
#[tauri::command]
fn get_app_paths() -> Result<AppPaths, String> {
    // Return:
    // - config_dir: ~/.rove/config
    // - state_dir: ~/.rove/state
    // - logs_dir: ~/.rove/logs
    // Never return API keys or tokens
}
```

**Security**: Only returns public directory paths. No secrets.

### `open_external`

```rust
#[tauri::command]
async fn open_external(url: String) -> Result<(), String> {
    // Validate URL scheme is http/https
    // Open in default browser
    // Block file://, javascript:, etc.
}
```

**Security**: URL validation before opening. No local file access.

### `show_in_folder`

```rust
#[tauri::command]
async fn show_in_folder(path: String) -> Result<(), String> {
    // Validate path is under a known workspace root
    // Open native file manager at that location
}
```

**Security**: Path validation against workspace roots. No arbitrary traversal.

---

## 6. Authentication and State

### Bearer Token Handling

1. **Generation**: On first launch, generate a random bearer token
2. **Storage**: Store in the platform config directory's `desktop.json` (0600 on Unix)
3. **Injection**: Inject document-start `window.__ROVE_TOKEN__` and
   `window.__ROVE_API_URL__` only for the Tauri/dev app origin
4. **API Calls**: Web UI includes `Authorization: Bearer <token>` in ordinary,
   SSE, and binary resource fetches

### ProductStore Location

- Desktop uses its platform user-data state directory for the API-global
  ProductStore; workspace runtime state remains under each selected workspace
  root.
- The Desktop config/state/log roots are explicit platform paths, not a hidden
  second ProductStore authority.
- Workspace selection respects existing `.rove/` state in each project

---

## 7. Platform Considerations

### Windows

- Bundle as `.msi` installer
- Store config/state in `%APPDATA%\Rove\`
- Handle Windows paths with backslashes correctly
- Test on Windows 10 and 11

### macOS

- Bundle as `.dmg` or `.app`
- Store config/state in `~/Library/Application Support/Rove/`
- Handle macOS security dialogs (filesystem access, etc.)
- Test on macOS 12+ (Intel and Apple Silicon)

### Linux

- Bundle as `.deb` and `.AppImage`
- Store config/state in `~/.config/rove/` and `~/.local/share/rove/`
- Follow XDG Base Directory spec
- Test on Ubuntu 22.04 and Fedora 38

---

## 8. Testing Strategy

### Unit Tests

- `api_server.rs`: Startup, shutdown, port allocation, health check
- `commands.rs`: Each Tauri command with valid/invalid inputs

### Integration Tests

- End-to-end Desktop launch → API ready → WebView loads
- Workspace selection flow
- Session creation and resume
- API authentication with injected token
- Graceful shutdown with in-flight requests

### Manual Testing (Platform Evidence)

- Install `.msi` / `.dmg` / `.deb` and launch
- Create workspace, start session, send message, receive response
- Open Settings, change provider, verify persistence
- Close and reopen, verify session resume
- Test on each platform (Windows, macOS, Linux)

---

## 9. Documentation Gate

Before implementing `apps/desktop`, this design must pass review for:

1. **No second backend**: Confirmed reuse of Runtime/API/Engine
2. **Security boundaries**: IPC allowlist, token handling, path validation
3. **Platform coverage**: Windows, macOS, Linux paths and packaging
4. **Crash recovery**: Explicit shutdown and error handling
5. **Testing plan**: Unit, integration, and manual platform tests

Once sealed, implementation proceeds in `apps/desktop/` with the structure 
defined above.

---

## 10. Implementation Checklist

- [x] Create `apps/desktop/` with Tauri 2 scaffold
- [x] Implement `api_server.rs` with embedded `rove-api` lifecycle
- [x] Implement allowlisted commands in `commands.rs`
- [x] Wire the native workspace picker into the shared Web workspace forms
- [x] Add bearer token generation and injection
- [x] Configure `tauri.conf.json` with CSP and security policies
- [x] Add application icons for all platforms
- [x] Write unit tests for server lifecycle and commands
- [ ] Write integration test for Desktop launch → API → WebView flow
- [ ] Test manual installation on Windows, macOS, Linux
- [x] Document platform-specific build and packaging steps
- [x] Verify no Desktop-only backend logic exists
- [x] Verify ProductStore ownership remains with API server
- [x] Update `docs/runtime/` with Desktop architecture
- [x] Pass the CP8 implementation gate; CP9 repository acceptance and PR
  evidence are recorded by the full-delivery handoff

---

## 11. Out of Scope (Post-D0)

- Auto-update via Tauri updater
- System tray and menu bar integration
- Native notifications
- Multi-window support
- Custom URL protocol handlers
- Code signing automation (manual for D0)
- macOS notarization automation (manual for D0)
- Windows installer silent mode
- Linux Flatpak or Snap packaging

These are explicitly deferred to post-D0 releases.
