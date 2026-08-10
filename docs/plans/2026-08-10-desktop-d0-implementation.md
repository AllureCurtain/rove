# Desktop D0 Implementation Plan

> Status: **Ready for execution**
>
> Date: 2026-08-10
>
> Design: [`../design/2026-08-10-desktop-d0-design.md`](../design/2026-08-10-desktop-d0-design.md)
>
> Branch: `program/full-delivery` (Checkpoint 8)

## 1. Prerequisites

- [ ] Design document sealed and reviewed
- [ ] Tauri CLI installed: `cargo install tauri-cli@2.1`
- [ ] Node.js 18+ and pnpm installed
- [ ] `apps/web` builds successfully
- [ ] `rove-api` runs successfully via CLI

## 2. Phase 1: Scaffold and Configuration (2h)

### Tasks

1. Create `apps/desktop/` directory structure
2. Initialize Tauri 2 project with `cargo tauri init`
3. Configure `tauri.conf.json` with security policies
4. Add application icons (512x512, 256x256, 128x128, 32x32, 16x16)
5. Configure Cargo.toml dependencies
6. Add build script for Web UI integration

### Success Criteria

- [ ] `cargo build -p rove-desktop` compiles
- [ ] `tauri.conf.json` includes CSP and localhost allowlist
- [ ] Icons exist for all required sizes
- [ ] Frontend dist points to `../web/dist`

### Files Changed

```
apps/desktop/Cargo.toml
apps/desktop/tauri.conf.json
apps/desktop/build.rs
apps/desktop/icons/*.png
apps/desktop/src/main.rs
apps/desktop/src/lib.rs
Cargo.toml (workspace member)
```

---

## 3. Phase 2: API Server Lifecycle (4h)

### Tasks

1. Implement `api_server.rs`:
   - Port allocation (bind to `localhost:0`)
   - Server startup with Runtime configuration
   - Health check polling
   - Graceful shutdown with timeout
   - Error handling and logging

2. Implement bearer token generation:
   - Generate random 32-byte token on first launch
   - Store in `~/.rove/config/desktop.json` with file permissions 0600
   - Load token on subsequent launches

3. Integrate with `main.rs`:
   - Start API server before WebView
   - Pass base URL to WebView
   - Handle server failures gracefully

### Success Criteria

- [ ] Server starts on random available port
- [ ] Health endpoint returns 200 OK
- [ ] Bearer token persists across restarts
- [ ] Server shuts down cleanly on app exit
- [ ] Panics log to `~/.rove/logs/desktop-crash.log`

### Files Changed

```
apps/desktop/src/api_server.rs
apps/desktop/src/main.rs
apps/desktop/src/config.rs
```

---

## 4. Phase 3: Tauri Commands (3h)

### Tasks

1. Implement `commands.rs`:
   - `workspace_select()` with native folder picker
   - `get_app_paths()` returning config/state/logs paths
   - `open_external(url)` with URL validation
   - `show_in_folder(path)` with path validation

2. Add security validation:
   - URL scheme allowlist (http, https only)
   - Path traversal checks
   - Workspace root validation

3. Register commands in `main.rs`

### Success Criteria

- [ ] Each command has unit tests
- [ ] Invalid inputs return typed errors
- [ ] No arbitrary filesystem or network access
- [ ] Commands work on all platforms

### Files Changed

```
apps/desktop/src/commands.rs
apps/desktop/src/validation.rs
apps/desktop/src/lib.rs
```

---

## 5. Phase 4: WebView Integration (3h)

### Tasks

1. Configure WebView window:
   - Set initial size (1400x900)
   - Set minimum size (800x600)
   - Enable dev tools in debug mode

2. Inject bearer token:
   - Add initialization script to inject token into localStorage
   - Format: `window.__ROVE_TOKEN__ = "<token>"`

3. Configure CSP:
   - Allow localhost connections
   - Allow WebSocket connections
   - Block external resources except API

4. Test Web UI loads correctly

### Success Criteria

- [ ] WebView loads `http://localhost:{port}`
- [ ] Bearer token is available in localStorage
- [ ] API calls include Authorization header
- [ ] Web UI functionality matches browser deployment

### Files Changed

```
apps/desktop/src/webview.rs
apps/desktop/tauri.conf.json
```

---

## 6. Phase 5: Platform Support (4h)

### Tasks

1. Implement platform-specific paths:
   - Windows: `%APPDATA%\Rove`
   - macOS: `~/Library/Application Support/Rove`
   - Linux: `~/.config/rove` and `~/.local/share/rove`

2. Test Windows path handling:
   - Backslash separators
   - UNC paths
   - Long path support

3. Test macOS security dialogs:
   - Filesystem access permissions
   - Notarization (manual, documented)

4. Test Linux XDG compliance:
   - Respect XDG_CONFIG_HOME
   - Respect XDG_DATA_HOME

### Success Criteria

- [ ] Paths resolve correctly on each platform
- [ ] Windows installer builds (`.msi`)
- [ ] macOS bundle builds (`.app` / `.dmg`)
- [ ] Linux packages build (`.deb`, `.AppImage`)

### Files Changed

```
apps/desktop/src/platform/mod.rs
apps/desktop/src/platform/windows.rs
apps/desktop/src/platform/macos.rs
apps/desktop/src/platform/linux.rs
```

---

## 7. Phase 6: Testing (5h)

### Unit Tests

```rust
// apps/desktop/src/api_server.rs
#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn server_starts_and_responds_to_health_check() { }
    
    #[tokio::test]
    async fn server_shuts_down_gracefully() { }
    
    #[test]
    fn port_allocation_finds_available_port() { }
}

// apps/desktop/src/commands.rs
#[cfg(test)]
mod tests {
    #[test]
    fn workspace_select_validates_absolute_paths() { }
    
    #[test]
    fn open_external_blocks_file_urls() { }
    
    #[test]
    fn show_in_folder_rejects_traversal() { }
}
```

### Integration Tests

```rust
// apps/desktop/tests/integration.rs
#[tokio::test]
async fn desktop_launches_api_and_loads_webview() { }

#[tokio::test]
async fn bearer_token_persists_across_restarts() { }

#[tokio::test]
async fn workspace_creation_and_session_resume() { }
```

### Manual Platform Tests

- [ ] Windows 10: Install `.msi`, launch, create session, verify persistence
- [ ] Windows 11: Same as above
- [ ] macOS 12+ (Intel): Install `.dmg`, launch, verify functionality
- [ ] macOS 12+ (Apple Silicon): Same as above
- [ ] Ubuntu 22.04: Install `.deb`, launch, verify functionality
- [ ] Fedora 38: Install `.AppImage`, launch, verify functionality

### Success Criteria

- [ ] All unit tests pass
- [ ] All integration tests pass
- [ ] Manual tests pass on each platform
- [ ] No crashes or hangs during normal operation

---

## 8. Phase 7: Documentation (2h)

### Tasks

1. Update `docs/runtime/README.md`:
   - Add Desktop architecture section
   - Document API server embedding
   - Document Tauri command allowlist

2. Create `apps/desktop/README.md`:
   - Development setup instructions
   - Build commands for each platform
   - Packaging instructions
   - Manual signing/notarization steps for D0

3. Update `docs/runtime/implementation-status.md`:
   - Mark Desktop D0 as implemented
   - Document platform coverage
   - Note deferred features (auto-update, etc.)

### Success Criteria

- [ ] Documentation matches implementation
- [ ] Build instructions are reproducible
- [ ] Platform-specific steps are documented

### Files Changed

```
docs/runtime/README.md
docs/runtime/implementation-status.md
apps/desktop/README.md
```

---

## 9. Phase 8: Acceptance Gate (2h)

### Checklist

- [ ] `cargo fmt --all --check` passes
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `cargo test --workspace` passes (including Desktop tests)
- [ ] `cargo build --release -p rove-desktop` succeeds
- [ ] `cargo tauri build` produces installers for current platform
- [ ] Web UI loads correctly in Desktop WebView
- [ ] Sessions persist across Desktop restarts
- [ ] No Desktop-only backend logic exists
- [ ] ProductStore ownership verified with API server
- [ ] Security validation passes (IPC allowlist, token handling, path checks)
- [ ] Manual platform test evidence collected

### Platform Build Commands

```bash
# Windows
cargo tauri build --target x86_64-pc-windows-msvc

# macOS (Intel)
cargo tauri build --target x86_64-apple-darwin

# macOS (Apple Silicon)
cargo tauri build --target aarch64-apple-darwin

# Linux
cargo tauri build --target x86_64-unknown-linux-gnu
```

---

## 10. Commit and Push

### Commit Message

```
feat(desktop): add Tauri 2 desktop application (D0)

Checkpoint 8: Desktop D0 and Tauri Delivery

- Create apps/desktop with Tauri 2 scaffold
- Implement embedded API server lifecycle with health checks
- Add allowlisted Tauri commands (workspace_select, get_app_paths,
  open_external, show_in_folder)
- Implement bearer token generation and WebView injection
- Add platform-specific path resolution (Windows/macOS/Linux)
- Configure CSP and security policies
- Reuse complete Runtime/API/Engine/ProductStore without duplication
- Add unit and integration tests for server lifecycle and commands
- Add manual platform test evidence
- Document Desktop architecture and build process

All tests pass. Desktop builds successfully on current platform.
Manual signing/notarization documented for D0.
```

### Push

```bash
git push origin program/full-delivery
```

---

## 11. Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Tauri 2 API changes | Pin exact version `2.1.x`, document upgrade path |
| Platform-specific bugs | Test on each platform, document known issues |
| WebView compatibility | Use same Web bundle as browser deployment |
| API server port conflicts | Random port allocation, handle bind failures |
| Token security | File permissions 0600, never log token value |
| Packaging complexity | Document manual steps, defer automation to post-D0 |

---

## 12. Estimated Timeline

- Phase 1: 2 hours
- Phase 2: 4 hours
- Phase 3: 3 hours
- Phase 4: 3 hours
- Phase 5: 4 hours
- Phase 6: 5 hours
- Phase 7: 2 hours
- Phase 8: 2 hours

**Total: ~25 hours** (approximately 3-4 days of focused work)

---

## 13. Success Metrics

- Desktop application launches on Windows, macOS, and Linux
- Sessions persist across Desktop restarts
- No crashes or hangs during normal operation
- No Desktop-only backend or state authority
- Security boundaries enforced (IPC allowlist, path validation)
- Documentation complete and reproducible
- CP8 gate passes with clean status