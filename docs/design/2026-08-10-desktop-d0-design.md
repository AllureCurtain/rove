# Desktop D0: Tauri 2 Integration Design

> Status: **Design sealed, ready for implementation**
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
│  └───────────────────────────────────────┘  │
└─────────────────────────────────────────────┘
         ↕ (localhost HTTP + WebSocket)
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

2. **Shared UI Bundle**: Desktop serves the exact `apps/web` build output. No 
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
tauri = { version = "2.1", features = ["protocol-asset"] }
rove-api = { path = "../../runtime" }
rove-config = { path = "../../packages/rove-config" }
tokio = { version = "1", features = ["full"] }
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
    "beforeDevCommand": "cd ../web && pnpm dev",
    "beforeBuildCommand": "cd ../web && pnpm build",
    "devUrl": "http://localhost:5173",
    "frontendDist": "../web/dist"
  },
  "app": {
    "windows": [
      {
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
      "csp": "default-src 'self'; connect-src 'self' http://localhost:* ws://localhost:*; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline' 'unsafe-eval'"
    }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": [
      "icons/icon.png"
    ]
  }
}
```

---

## 4. API Server Lifecycle

### Startup Sequence

1. **Parse Configuration**: Load bearer token, CORS, state root from config/env
2. **Find Available Port**: Bind to random `localhost:0`, get assigned port
3. **Start API Server**: Launch `rove-api` with runtime configuration
4. **Wait for Health**: Poll `http://localhost:{port}/health` until ready
5. **Launch WebView**: Pass `http://localhost:{port}` as base URL
6. **Inject Token**: Store bearer token in WebView localStorage on first load

### Shutdown Sequence

1. **Window Close**: User closes the window or quits the app
2. **Cancel API Server**: Send cancellation signal to API server task
3. **Graceful Drain**: Wait up to 5 seconds for in-flight requests to complete
4. **Force Kill**: If timeout expires, drop the server task
5. **Clean Temp State**: Remove any temp files created during this session

### Crash Recovery

- If API server panics or exits unexpectedly, show error dialog and exit
- Do NOT auto-restart the server in a loop (prevents runaway failures)
- Log crash details to `~/.rove/logs/desktop-crash.log`

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
2. **Storage**: Store in `~/.rove/config/desktop.json` (file permissions 0600)
3. **Injection**: On WebView load, inject into localStorage via `window.__ROVE_TOKEN__`
4. **API Calls**: Web UI includes `Authorization: Bearer <token>` in all requests

### ProductStore Location

- Desktop uses the same `~/.rove/state/` as CLI
- No Desktop-specific state directory
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

- [ ] Create `apps/desktop/` with Tauri 2 scaffold
- [ ] Implement `api_server.rs` with embedded `rove-api` lifecycle
- [ ] Implement allowlisted commands in `commands.rs`
- [ ] Add bearer token generation and injection
- [ ] Configure `tauri.conf.json` with CSP and security policies
- [ ] Add application icons for all platforms
- [ ] Write unit tests for server lifecycle and commands
- [ ] Write integration test for Desktop launch → API → WebView flow
- [ ] Test manual installation on Windows, macOS, Linux
- [ ] Document platform-specific build and packaging steps
- [ ] Verify no Desktop-only backend logic exists
- [ ] Verify ProductStore ownership remains with API server
- [ ] Update `docs/runtime/` with Desktop architecture
- [ ] Pass CP8 gate and commit

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

