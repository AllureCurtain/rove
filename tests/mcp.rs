use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.pop();
    root
}

fn workspace_path(rel: impl AsRef<Path>) -> PathBuf {
    workspace_root().join(rel)
}

fn workspace_path_string(rel: impl AsRef<Path>) -> String {
    workspace_path(rel).to_string_lossy().into_owned()
}

use rove_core::ToolContentBlock;
use rove_core::ToolError;
use rove_core::ToolErrorDomain;
use rove_core::ToolRegistry;
use rove_core::ToolResultOutcome;
use rove_runtime::environment::{ExecutionEnvironment, local_environment};
use rove_runtime::memory::paths::MemoryPaths;
use rove_runtime::state::tool_artifacts::ToolArtifactStore;
use rove_runtime::tools::mcp_proxy::{
    MAX_MCP_RESPONSE_BYTES, McpProbeFailureKind, McpRuntimeState, McpServerConfig,
    McpServerHealthStatus, McpTransport, McpTransportPolicy, probe_mcp_server, register_mcp_tools,
    register_mcp_tools_with_environment, resolve_mcp_server_environment,
};
use rove_runtime::tools::runtime_context::{
    runtime_tool_context, runtime_tool_context_with_artifacts,
};
use rove_runtime::types::{ApprovalPolicy, ToolContext};
use rove_runtime::workspace::Workspace;
use std::collections::HashMap;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

static MCP_STDIO_TEST_LOCK: Mutex<()> = Mutex::const_new(());

#[tokio::test]
async fn mcp_sse_rejects_oversized_discovery_and_json_responses() {
    use axum::Router;
    use axum::http::header::CONTENT_TYPE;
    use axum::routing::{get, post};

    for oversized_discovery in [true, false] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let oversized = "x".repeat(MAX_MCP_RESPONSE_BYTES + 1);
        let router = if oversized_discovery {
            Router::new().route(
                "/sse",
                get(move || {
                    let body = oversized.clone();
                    async move { ([(CONTENT_TYPE, "text/event-stream")], body) }
                }),
            )
        } else {
            Router::new()
                .route(
                    "/sse",
                    get(|| async {
                        ([(CONTENT_TYPE, "text/event-stream")], "data: /messages\n\n")
                    }),
                )
                .route(
                    "/messages",
                    post(move || {
                        let body = oversized.clone();
                        async move { ([(CONTENT_TYPE, "application/json")], body) }
                    }),
                )
        };
        let server_task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        let failure = probe_mcp_server(McpServerConfig {
            name: "bounded-sse".to_string(),
            enabled: true,
            required: true,
            transport: McpTransport::Sse,
            command: String::new(),
            args: Vec::new(),
            env: Default::default(),
            env_names: Vec::new(),
            url: format!("http://{address}/sse"),
            policy: responsive_mcp_policy(),
        })
        .await
        .unwrap_err();

        assert_eq!(failure.kind, McpProbeFailureKind::Protocol);
        server_task.abort();
    }
}

fn python_command() -> String {
    if cfg!(windows) {
        "python".to_string()
    } else {
        "python3".to_string()
    }
}

fn mcp_context<'a>(workspace: &'a Workspace) -> ToolContext<'a> {
    runtime_tool_context(
        rove_runtime::types::CallId::new(),
        workspace,
        MemoryPaths::from_workspace(workspace, 8),
        ApprovalPolicy::Auto,
        None,
        CancellationToken::new(),
    )
}

fn short_mcp_policy() -> McpTransportPolicy {
    McpTransportPolicy {
        request_timeout_ms: 250,
        stderr_capture_bytes: 4096,
    }
}

fn responsive_mcp_policy() -> McpTransportPolicy {
    McpTransportPolicy {
        request_timeout_ms: 2_000,
        stderr_capture_bytes: 4096,
    }
}

#[tokio::test]
async fn mcp_proxy_registers_and_calls_stdio_tools() {
    let _guard = MCP_STDIO_TEST_LOCK.lock().await;
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut registry = ToolRegistry::new();
    let count = register_mcp_tools(
        &mut registry,
        vec![McpServerConfig {
            name: "mock-server".to_string(),
            enabled: true,
            required: true,
            transport: McpTransport::Stdio,
            command: python_command(),
            args: vec![workspace_path_string("tests/fixtures/mcp_mock_server.py").to_string()],
            env: Default::default(),
            env_names: Vec::new(),
            url: String::new(),
            policy: McpTransportPolicy::default(),
        }],
    )
    .await
    .unwrap();

    assert_eq!(count, 2);
    assert!(registry.has("mcp__mock_server__echo_remote"));
    assert!(registry.has("mcp__mock_server__delete_remote"));
    let remotely_claimed_read_only = registry
        .descriptor("mcp__mock_server__echo_remote")
        .unwrap();
    assert!(remotely_claimed_read_only.destructive);
    assert!(!remotely_claimed_read_only.parallel_safe);
    assert!(
        registry
            .descriptor("mcp__mock_server__delete_remote")
            .unwrap()
            .destructive
    );

    let output = registry
        .execute(
            "mcp__mock_server__echo_remote",
            serde_json::json!({ "message": "hello" }),
            &mcp_context(&workspace),
        )
        .await
        .unwrap();

    assert_eq!(output.content, "remote: hello");
    let runtime_identity = registry
        .extension::<McpRuntimeState>()
        .and_then(|state| state.snapshot("mock-server"))
        .expect("MCP runtime snapshot");
    let result_identity = output
        .envelope
        .as_ref()
        .and_then(|envelope| envelope.protocol_metadata.server_identity_hash.as_deref())
        .expect("MCP result identity");
    assert_eq!(result_identity, runtime_identity.server_identity_hash);
}

#[tokio::test]
async fn required_mcp_stdio_activation_timeout_fails_closed_without_raw_diagnostics() {
    let _guard = MCP_STDIO_TEST_LOCK.lock().await;
    let mut registry = ToolRegistry::new();
    let err = register_mcp_tools(
        &mut registry,
        vec![McpServerConfig {
            name: "hanging-server".to_string(),
            enabled: true,
            required: true,
            transport: McpTransport::Stdio,
            command: python_command(),
            args: vec![workspace_path_string("tests/fixtures/mcp_hanging_server.py").to_string()],
            env: Default::default(),
            env_names: Vec::new(),
            url: String::new(),
            policy: short_mcp_policy(),
        }],
    )
    .await
    .unwrap_err();

    let message = err.to_string();
    assert!(
        message.contains("required MCP server `hanging-server`"),
        "{message}"
    );
    assert!(message.contains("mcp_activation_timeout"), "{message}");
    assert!(!message.contains("hanging server received"), "{message}");
    assert!(!message.contains("250ms"), "{message}");
}

#[tokio::test]
async fn mcp_tool_call_error_maps_to_structured_tool_error() {
    let _guard = MCP_STDIO_TEST_LOCK.lock().await;
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut registry = ToolRegistry::new();
    register_mcp_tools(
        &mut registry,
        vec![McpServerConfig {
            name: "error-server".to_string(),
            enabled: true,
            required: true,
            transport: McpTransport::Stdio,
            command: python_command(),
            args: vec![workspace_path_string("tests/fixtures/mcp_error_server.py").to_string()],
            env: Default::default(),
            env_names: Vec::new(),
            url: String::new(),
            policy: responsive_mcp_policy(),
        }],
    )
    .await
    .unwrap();

    let err = registry
        .execute(
            "mcp__error_server__fail_remote",
            serde_json::json!({}),
            &mcp_context(&workspace),
        )
        .await
        .unwrap_err();

    match err {
        ToolError::ExecutionFailed { reason } => {
            assert_eq!(reason, "MCP JSON-RPC error -32000: remote boom");
        }
        other => panic!("expected MCP execution failure, got {other:?}"),
    }
}

#[tokio::test]
async fn dropping_stdio_mcp_registry_cleans_up_child_process() {
    let _guard = MCP_STDIO_TEST_LOCK.lock().await;
    let tmp = tempfile::TempDir::new().unwrap();
    let pid_path = tmp.path().join("mcp.pid");
    let mut env = HashMap::new();
    env.insert(
        "ROVE_MCP_TEST_PID_FILE".to_string(),
        pid_path.to_string_lossy().to_string(),
    );

    {
        let mut registry = ToolRegistry::new();
        register_mcp_tools(
            &mut registry,
            vec![McpServerConfig {
                name: "lifecycle-server".to_string(),
                enabled: true,
                required: true,
                transport: McpTransport::Stdio,
                command: python_command(),
                args: vec![
                    workspace_path_string("tests/fixtures/mcp_lifecycle_server.py").to_string(),
                ],
                env,
                env_names: Vec::new(),
                url: String::new(),
                policy: responsive_mcp_policy(),
            }],
        )
        .await
        .unwrap();
        assert!(registry.has("mcp__lifecycle_server__ping_remote"));
    }

    let pid: u32 = std::fs::read_to_string(&pid_path)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_process_exits(pid, Duration::from_secs(3));
}

#[tokio::test]
async fn disabled_mcp_servers_are_never_assembled_or_environment_resolved() {
    let _guard = MCP_STDIO_TEST_LOCK.lock().await;
    let mut registry = ToolRegistry::new();
    // A disabled server must be skipped before environment resolution and spawn,
    // so an unavailable variable and a bogus command must not fail assembly.
    let count = register_mcp_tools(
        &mut registry,
        vec![McpServerConfig {
            name: "disabled_server".to_string(),
            enabled: false,
            required: true,
            transport: McpTransport::Stdio,
            command: "rove-command-that-does-not-exist-019fcfd2".to_string(),
            args: Vec::new(),
            env: Default::default(),
            env_names: vec!["ROVE_MCP_ENV_MISSING_019FCFD2".to_string()],
            url: String::new(),
            policy: short_mcp_policy(),
        }],
    )
    .await
    .unwrap();

    assert_eq!(count, 0);
    assert!(!registry.has("mcp__disabled_server__echo_remote"));
    assert!(
        registry
            .descriptors()
            .iter()
            .all(|descriptor| !descriptor.name.starts_with("mcp__disabled_server__"))
    );
}

#[tokio::test]
async fn an_optional_server_failure_degrades_without_removing_local_tools() {
    let _guard = MCP_STDIO_TEST_LOCK.lock().await;
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(rove_runtime::tools::echo::EchoTool));

    let registered = register_mcp_tools(
        &mut registry,
        vec![McpServerConfig {
            name: "optional_missing".to_string(),
            enabled: true,
            required: false,
            transport: McpTransport::Stdio,
            command: "rove-command-that-does-not-exist-optional".to_string(),
            args: Vec::new(),
            env: Default::default(),
            env_names: Vec::new(),
            url: String::new(),
            policy: short_mcp_policy(),
        }],
    )
    .await
    .unwrap();

    assert_eq!(registered, 0);
    assert!(registry.has("echo"));
    let state = registry.extension::<McpRuntimeState>().unwrap();
    let snapshot = state.snapshot("optional_missing").unwrap();
    assert!(!snapshot.required);
    assert_eq!(snapshot.status, McpServerHealthStatus::Degraded);
    assert_eq!(snapshot.tool_count, 0);
    assert!(snapshot.failure_code.is_some());
}

#[test]
fn mcp_environment_resolution_bounds_the_name_and_the_missing_case() {
    let base = McpServerConfig {
        name: "env_server".to_string(),
        enabled: true,
        required: true,
        transport: McpTransport::Stdio,
        command: python_command(),
        args: Vec::new(),
        env: Default::default(),
        env_names: Vec::new(),
        url: String::new(),
        policy: short_mcp_policy(),
    };

    for invalid in ["BAD-NAME", "1LEADING_DIGIT", "", "HAS SPACE", "HAS=EQUALS"] {
        let mut server = base.clone();
        server.env_names = vec![invalid.to_string()];
        let error = resolve_mcp_server_environment(server).unwrap_err();
        assert!(
            error.to_string().contains("invalid"),
            "unexpected error for {invalid:?}: {error}"
        );
    }

    let mut missing = base;
    missing.env_names = vec!["ROVE_MCP_ENV_MISSING_019FCFD2".to_string()];
    let error = resolve_mcp_server_environment(missing).unwrap_err();
    assert!(
        error.to_string().contains("unavailable"),
        "unexpected error: {error}"
    );

    // This test deliberately stops at the two cases that need no environment
    // value. The value path — injected and registered, refused for length, and
    // refused by the authority past its cap — is exercised by
    // `tools::mcp_proxy::environment_resolution_tests` in the runtime, where the
    // lookup and the authority are injected: mutating this process's environment
    // from a test binary that runs in parallel would race every other test here,
    // and the real registry cannot be filled to its cap without disarming
    // redaction for the rest of the binary.
}

/// The third-party server this opt-in gate drives, pinned by revision.
///
/// The tool-set assertion below belongs to one published version; a different
/// revision would legitimately publish a different set, and that is exactly
/// what the assertion reports.
const OFFICIAL_FILESYSTEM_MCP_PACKAGE: &str = "@modelcontextprotocol/server-filesystem@2026.8.31";

const OFFICIAL_FILESYSTEM_SERVER_NAME: &str = "official_filesystem";

/// The tools `@modelcontextprotocol/server-filesystem@2026.8.31` published over
/// `tools/list` (serverInfo `secure-filesystem-server` 0.2.0, negotiated
/// protocol `2025-06-18`), as discovered through rove's stdio proxy.
const OFFICIAL_FILESYSTEM_MCP_TOOLS: [&str; 14] = [
    "create_directory",
    "directory_tree",
    "edit_file",
    "get_file_info",
    "list_allowed_directories",
    "list_directory",
    "list_directory_with_sizes",
    "move_file",
    "read_file",
    "read_media_file",
    "read_multiple_files",
    "read_text_file",
    "search_files",
    "write_file",
];

/// The second real third-party server this change drives.
///
/// Pinned by revision for the same reason as the filesystem server. It is
/// driven through the identical stdio path; it was chosen because every schema
/// it publishes through `tools/list` fits rove's bounded subset, so the whole
/// interoperability path really executes instead of stopping at a refusal.
const OFFICIAL_MEMORY_MCP_PACKAGE: &str = "@modelcontextprotocol/server-memory@2026.8.31";

const OFFICIAL_MEMORY_SERVER_NAME: &str = "official_memory";

/// The tools `@modelcontextprotocol/server-memory@2026.8.31` published over
/// `tools/list` (negotiated protocol `2025-06-18`), as discovered through
/// rove's stdio proxy.
const OFFICIAL_MEMORY_MCP_TOOLS: [&str; 9] = [
    "add_observations",
    "create_entities",
    "create_relations",
    "delete_entities",
    "delete_observations",
    "delete_relations",
    "open_nodes",
    "read_graph",
    "search_nodes",
];

/// A real 1x1 PNG, so the artifact read back out of the durable store is
/// genuine image bytes rather than a blob that merely decodes.
const PNG_1X1: [u8; 70] = [
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 218, 99, 252, 207, 192, 240, 31, 0,
    5, 0, 1, 255, 171, 206, 54, 137, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

/// The stdio launcher for a pinned npm MCP server, confined to `root`.
///
/// Windows ships `npx` as the `npx.cmd` batch shim, which is a script rather
/// than an executable image, so the default launcher runs it through
/// `cmd /C npx` instead of relying on the runner's batch-file handling. Both
/// override variables keep their previous meaning: `<prefix>_COMMAND` replaces
/// the launcher, and `<prefix>_ARGS` replaces its whole argument vector (in
/// which case the caller must pass the confined root too).
///
/// `root` is passed as a trailing argument even to servers that ignore it, so
/// this run's unique temporary root appears in every hop's command line and in
/// no other process on the machine; the teardown assertion in this file matches
/// on exactly that.
fn npx_stdio_launch(
    package: &str,
    root: &Path,
    command_var: &str,
    args_var: &str,
) -> (String, Vec<String>) {
    let confined = root.to_string_lossy().into_owned();
    let npx_args = || vec!["-y".to_string(), package.to_string(), confined.clone()];
    let default = if cfg!(windows) {
        let mut args = vec!["/C".to_string(), "npx".to_string()];
        args.extend(npx_args());
        ("cmd".to_string(), args)
    } else {
        ("npx".to_string(), npx_args())
    };
    let command = std::env::var(command_var).unwrap_or(default.0);
    let args = std::env::var(args_var)
        .ok()
        .map(|value| value.split_whitespace().map(str::to_string).collect())
        .unwrap_or(default.1);
    (command, args)
}

fn official_filesystem_launch(allowed_dir: &Path) -> (String, Vec<String>) {
    npx_stdio_launch(
        OFFICIAL_FILESYSTEM_MCP_PACKAGE,
        allowed_dir,
        "ROVE_MCP_FILESYSTEM_COMMAND",
        "ROVE_MCP_FILESYSTEM_ARGS",
    )
}

fn official_memory_launch(workspace_root: &Path) -> (String, Vec<String>) {
    npx_stdio_launch(
        OFFICIAL_MEMORY_MCP_PACKAGE,
        workspace_root,
        "ROVE_MCP_MEMORY_COMMAND",
        "ROVE_MCP_MEMORY_ARGS",
    )
}

/// The stdio configuration that drives the official server through rove.
fn official_filesystem_config(allowed_dir: &Path) -> McpServerConfig {
    let (command, args) = official_filesystem_launch(allowed_dir);
    println!("MCP_INTEROP launch: {command} {args:?}");
    McpServerConfig {
        name: OFFICIAL_FILESYSTEM_SERVER_NAME.to_string(),
        enabled: true,
        required: true,
        transport: McpTransport::Stdio,
        command,
        args,
        env: Default::default(),
        env_names: Vec::new(),
        url: String::new(),
        // The first `npx` run of a revision installs it, so the activation
        // budget is generous; the outer test budget still bounds the gate.
        policy: McpTransportPolicy {
            request_timeout_ms: 120_000,
            stderr_capture_bytes: 8_192,
        },
    }
}

/// The stdio configuration that drives the official memory server through rove.
///
/// The server persists the knowledge graph it is given, so this run points it at
/// a file inside its own temporary workspace and never at a user or shared path.
fn official_memory_config(workspace_root: &Path, memory_file: &Path) -> McpServerConfig {
    let (command, args) = official_memory_launch(workspace_root);
    McpServerConfig {
        name: OFFICIAL_MEMORY_SERVER_NAME.to_string(),
        enabled: true,
        required: true,
        transport: McpTransport::Stdio,
        command,
        args,
        env: HashMap::from([(
            "MEMORY_FILE_PATH".to_string(),
            memory_file.to_string_lossy().into_owned(),
        )]),
        env_names: Vec::new(),
        url: String::new(),
        policy: McpTransportPolicy {
            request_timeout_ms: 120_000,
            stderr_capture_bytes: 8_192,
        },
    }
}

/// A tool context carrying a durable artifact authority, as a real run has.
fn mcp_artifact_context<'a>(
    workspace: &'a Workspace,
    store: Arc<ToolArtifactStore>,
) -> ToolContext<'a> {
    runtime_tool_context_with_artifacts(
        rove_runtime::types::CallId::new(),
        workspace,
        MemoryPaths::from_workspace(workspace, 8),
        ApprovalPolicy::Auto,
        None,
        CancellationToken::new(),
        local_environment(workspace),
        Some(store),
    )
}

/// Interoperability with the official filesystem MCP server, driven through
/// rove's own stdio proxy: real discovery, real calls, real refusals, real
/// artifact bytes, and a real child-process teardown.
///
/// This is opt-in because it needs `npx` and the npm registry. Without
/// `ROVE_MCP_FILESYSTEM_SMOKE=1` it only proves the skip path.
#[tokio::test]
async fn mcp_official_filesystem_server_smoke_when_enabled() {
    if std::env::var("ROVE_MCP_FILESYSTEM_SMOKE").ok().as_deref() != Some("1") {
        eprintln!(
            "skipping official filesystem MCP interoperability; set ROVE_MCP_FILESYSTEM_SMOKE=1 to run"
        );
        return;
    }
    let _guard = MCP_STDIO_TEST_LOCK.lock().await;

    timeout(
        Duration::from_secs(300),
        official_filesystem_interoperability(),
    )
    .await
    .expect("the official filesystem MCP gate exceeded its bounded budget");
}

async fn official_filesystem_interoperability() {
    let temp = tempfile::TempDir::new().unwrap();
    let allowed_dir = normalize_windows_extended_path(temp.path().canonicalize().unwrap());
    let outside = tempfile::TempDir::new().unwrap();
    let outside_dir = normalize_windows_extended_path(outside.path().canonicalize().unwrap());

    let note = "hello from the official filesystem MCP server\n";
    let outside_secret = "outside the allowed root\n";
    std::fs::write(allowed_dir.join("note.txt"), note).unwrap();
    std::fs::create_dir(allowed_dir.join("sub")).unwrap();
    std::fs::write(allowed_dir.join("sub").join("nested.txt"), "nested\n").unwrap();
    std::fs::write(allowed_dir.join("dot.png"), PNG_1X1).unwrap();
    std::fs::write(outside_dir.join("outside.txt"), outside_secret).unwrap();

    let workspace = Workspace::detect(&allowed_dir).unwrap();
    let config = official_filesystem_config(&allowed_dir);
    let environment = local_environment(&workspace);

    let mut registry = ToolRegistry::new();
    let count = match register_mcp_tools_with_environment(
        &mut registry,
        vec![config.clone()],
        Arc::clone(&environment),
    )
    .await
    {
        Ok(count) => count,
        Err(registration) => {
            // Rove refused the server's published catalog. A coarse failure code
            // is not evidence of why, so drive the same proxy directly for the
            // exact protocol diagnostic, then record the blocker loudly.
            let refusal = official_filesystem_catalog_refusal(
                config,
                Arc::clone(&environment),
                &allowed_dir.to_string_lossy(),
            )
            .await;
            record_official_filesystem_blocker(&registration, &refusal);
            return;
        }
    };

    // --- Discovery: the tool set comes from the server, not from this file ---
    let namespace = format!("mcp__{OFFICIAL_FILESYSTEM_SERVER_NAME}__");
    let mut discovered: Vec<String> = registry
        .descriptors()
        .iter()
        .filter_map(|descriptor| descriptor.name.strip_prefix(&namespace).map(str::to_string))
        .collect();
    discovered.sort();
    println!(
        "MCP_INTEROP tools/list discovered {} tools: {discovered:?}",
        discovered.len()
    );
    assert_eq!(
        count,
        OFFICIAL_FILESYSTEM_MCP_TOOLS.len(),
        "registered tool count must match the published catalog: {discovered:?}"
    );
    assert_eq!(
        discovered, OFFICIAL_FILESYSTEM_MCP_TOOLS,
        "the server's published tool set changed; re-record it with the new package revision"
    );

    // Remote annotations are intent, never a local permission grant.
    for name in &discovered {
        let descriptor = registry
            .descriptor(&format!("{namespace}{name}"))
            .expect("every discovered tool has a descriptor");
        assert!(
            descriptor.destructive && !descriptor.parallel_safe,
            "{name} must stay destructive and not parallel-safe despite the server's readOnlyHint"
        );
    }

    let state = registry
        .extension::<McpRuntimeState>()
        .expect("MCP runtime state");
    let snapshot = state
        .snapshot(OFFICIAL_FILESYSTEM_SERVER_NAME)
        .expect("MCP runtime snapshot");
    assert_eq!(snapshot.status, McpServerHealthStatus::Ready);
    assert_eq!(snapshot.tool_count, OFFICIAL_FILESYSTEM_MCP_TOOLS.len());
    assert_eq!(snapshot.protocol_version.as_deref(), Some("2025-06-18"));
    assert!(snapshot.catalog_hash.is_some());
    assert!(snapshot.failure_code.is_none());
    println!(
        "MCP_INTEROP negotiated protocol {} catalog {} identity {}",
        snapshot.protocol_version.as_deref().unwrap_or("none"),
        snapshot.catalog_hash.as_deref().unwrap_or("none"),
        snapshot.server_identity_hash
    );

    let store = Arc::new(ToolArtifactStore::new(
        temp.path().join("runs").join("run_mcp"),
    ));
    let context = mcp_artifact_context(&workspace, Arc::clone(&store));

    // --- Real call: the server's own view of its allowed roots ---------------
    let allowed = execute_checked(
        &registry,
        &format!("{namespace}list_allowed_directories"),
        serde_json::json!({}),
        &context,
    )
    .await;
    assert!(
        allowed
            .content
            .to_lowercase()
            .contains(&allowed_dir.to_string_lossy().to_lowercase()),
        "the allowed root must be the temporary directory this test created: {}",
        allowed.content
    );

    // --- Real call: list a directory this test created -----------------------
    let listing = execute_checked(
        &registry,
        &format!("{namespace}list_directory"),
        serde_json::json!({ "path": allowed_dir }),
        &context,
    )
    .await;
    println!("MCP_INTEROP list_directory -> {}", listing.content);
    for expected in ["note.txt", "sub", "dot.png"] {
        assert!(
            listing.content.contains(expected),
            "the directory listing must contain {expected}: {}",
            listing.content
        );
    }

    // --- Real call: read a file with known content ---------------------------
    let read = execute_checked(
        &registry,
        &format!("{namespace}read_text_file"),
        serde_json::json!({ "path": allowed_dir.join("note.txt") }),
        &context,
    )
    .await;
    println!("MCP_INTEROP read_text_file -> {}", read.content);
    assert!(
        read.content.contains(note.trim()),
        "the file content must round-trip: {}",
        read.content
    );
    let read_envelope = read.envelope.as_ref().expect("an envelope is produced");
    assert_eq!(read_envelope.outcome, ToolResultOutcome::Success);
    assert_eq!(
        read_envelope.protocol_metadata.remote_tool_name.as_deref(),
        Some("read_text_file")
    );
    assert_eq!(
        read_envelope
            .protocol_metadata
            .server_identity_hash
            .as_deref(),
        Some(snapshot.server_identity_hash.as_str())
    );
    // The server declares an outputSchema for this tool, so the real
    // structuredContent was validated rather than trusted.
    let structured = read_envelope
        .structured_content
        .as_ref()
        .expect("the server returned structured content");
    assert_eq!(
        structured.schema_valid,
        Some(true),
        "structured content must satisfy the declared schema: {:?}",
        structured.schema_error
    );

    // --- Real call: media bytes into the durable artifact store --------------
    let media = execute_checked(
        &registry,
        &format!("{namespace}read_media_file"),
        serde_json::json!({ "path": allowed_dir.join("dot.png") }),
        &context,
    )
    .await;
    let media_envelope = media.envelope.as_ref().expect("an envelope is produced");
    assert_eq!(media_envelope.outcome, ToolResultOutcome::Success);
    assert_eq!(
        media_envelope.artifacts.len(),
        1,
        "the image block becomes exactly one artifact"
    );
    let ToolContentBlock::Image { artifact, .. } = &media_envelope.content_blocks[0] else {
        panic!(
            "expected an image block, got {:?}",
            media_envelope.content_blocks
        );
    };
    assert_eq!(artifact.mime_type.as_deref(), Some("image/png"));
    assert_eq!(artifact.byte_length, PNG_1X1.len() as u64);
    assert!(
        !media_envelope.content_blocks[0]
            .model_text()
            .contains("iVBORw0KGgo"),
        "base64 must not reach the model projection of the block"
    );
    let bytes = store.get(&artifact.artifact_id).await.unwrap();
    assert_eq!(
        bytes.as_slice(),
        PNG_1X1.as_slice(),
        "the retained artifact must be the exact bytes this test wrote"
    );
    let ledger = store.ledger().await.unwrap();
    assert_eq!(ledger.len(), 1, "one committed artifact entry: {ledger:?}");
    println!(
        "MCP_INTEROP read_media_file artifact {} ({} bytes, {})",
        artifact.artifact_id.as_str(),
        artifact.byte_length,
        artifact.mime_type.as_deref().unwrap_or("unknown")
    );

    // --- Negative: an unknown tool name fails closed, typed ------------------
    let unknown = registry
        .execute(
            &format!("{namespace}definitely_not_a_tool"),
            serde_json::json!({}),
            &context,
        )
        .await
        .expect_err("an unknown tool name must not resolve");
    assert!(
        matches!(unknown, ToolError::UnknownTool { ref name } if name.ends_with("definitely_not_a_tool")),
        "expected a typed unknown-tool error, got {unknown:?}"
    );

    // --- Negative: the server's own schema is enforced before dispatch -------
    let invalid = registry
        .execute(
            &format!("{namespace}read_text_file"),
            serde_json::json!({}),
            &context,
        )
        .await
        .expect_err("a missing required argument must fail before dispatch");
    assert!(
        matches!(invalid, ToolError::InvalidArgs { .. }),
        "expected a typed argument error, got {invalid:?}"
    );

    // --- Negative: a path outside the allowed root is refused ----------------
    assert_remote_refusal(
        &registry,
        &format!("{namespace}read_text_file"),
        serde_json::json!({ "path": outside_dir.join("outside.txt") }),
        &context,
        "access denied",
        outside_secret.trim(),
        "an absolute path outside the allowed root",
    )
    .await;

    // --- Negative: a traversal-shaped path is refused, not followed ----------
    let escaped = allowed_dir.join("..").join("outside.txt");
    assert_remote_refusal(
        &registry,
        &format!("{namespace}read_text_file"),
        serde_json::json!({ "path": escaped }),
        &context,
        "access denied",
        outside_secret.trim(),
        "a traversal-shaped path",
    )
    .await;

    // --- Boundedness: the third-party process tree dies with the registry ----
    let marker = allowed_dir.to_string_lossy().into_owned();
    drop(context);
    assert_process_tree_dies(&marker, || drop(registry)).await;
}

/// Real interoperability with a second third-party MCP server, driven through
/// rove's own stdio proxy: real discovery, real calls, a real remote refusal,
/// real fail-closed argument validation, a real mutation round trip, confined
/// persistence, and a real child-process teardown.
///
/// This is opt-in because it needs `npx` and the npm registry. Without
/// `ROVE_MCP_MEMORY_SMOKE=1` it only proves the skip path.
#[tokio::test]
async fn mcp_official_memory_server_interoperability_when_enabled() {
    if std::env::var("ROVE_MCP_MEMORY_SMOKE").ok().as_deref() != Some("1") {
        eprintln!(
            "skipping official memory MCP interoperability; set ROVE_MCP_MEMORY_SMOKE=1 to run"
        );
        return;
    }
    let _guard = MCP_STDIO_TEST_LOCK.lock().await;

    timeout(Duration::from_secs(300), official_memory_interoperability())
        .await
        .expect("the official memory MCP gate exceeded its bounded budget");
}

async fn official_memory_interoperability() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace_root = normalize_windows_extended_path(temp.path().canonicalize().unwrap());
    let memory_file = workspace_root.join("memory.json");
    let workspace = Workspace::detect(&workspace_root).unwrap();
    let environment = local_environment(&workspace);
    let config = official_memory_config(&workspace_root, &memory_file);
    println!(
        "MCP_INTEROP memory launch: {} {:?}",
        config.command, config.args
    );

    let mut registry = ToolRegistry::new();
    let count =
        register_mcp_tools_with_environment(&mut registry, vec![config], Arc::clone(&environment))
            .await
            .expect("the official memory server must activate through the stdio proxy");
    let namespace = format!("mcp__{OFFICIAL_MEMORY_SERVER_NAME}__");

    // --- Discovery: the tool set comes from the server, not from this file ---
    let mut discovered: Vec<String> = registry
        .descriptors()
        .iter()
        .filter_map(|descriptor| descriptor.name.strip_prefix(&namespace).map(str::to_string))
        .collect();
    discovered.sort();
    println!(
        "MCP_INTEROP memory tools/list discovered {} tools: {discovered:?}",
        discovered.len()
    );
    assert_eq!(
        count,
        OFFICIAL_MEMORY_MCP_TOOLS.len(),
        "registered tool count must match the published catalog: {discovered:?}"
    );
    assert_eq!(
        discovered, OFFICIAL_MEMORY_MCP_TOOLS,
        "the server's published tool set changed; re-record it with the new package revision"
    );

    // Remote annotations are intent, never a local permission grant.
    for name in &discovered {
        let descriptor = registry
            .descriptor(&format!("{namespace}{name}"))
            .expect("every discovered tool has a descriptor");
        assert!(
            descriptor.destructive && !descriptor.parallel_safe,
            "{name} must stay destructive and not parallel-safe despite the server's readOnlyHint"
        );
    }

    let state = registry
        .extension::<McpRuntimeState>()
        .expect("MCP runtime state");
    let snapshot = state
        .snapshot(OFFICIAL_MEMORY_SERVER_NAME)
        .expect("MCP runtime snapshot");
    assert_eq!(snapshot.status, McpServerHealthStatus::Ready);
    assert_eq!(snapshot.tool_count, OFFICIAL_MEMORY_MCP_TOOLS.len());
    assert_eq!(snapshot.protocol_version.as_deref(), Some("2025-06-18"));
    assert!(snapshot.catalog_hash.is_some());
    assert!(snapshot.failure_code.is_none());
    println!(
        "MCP_INTEROP memory negotiated protocol {} catalog {} identity {}",
        snapshot.protocol_version.as_deref().unwrap_or("none"),
        snapshot.catalog_hash.as_deref().unwrap_or("none"),
        snapshot.server_identity_hash
    );

    let store = Arc::new(ToolArtifactStore::new(
        temp.path().join("runs").join("run_memory"),
    ));
    let context = mcp_artifact_context(&workspace, Arc::clone(&store));

    // --- Real call: write real state -----------------------------------------
    let created = execute_checked(
        &registry,
        &format!("{namespace}create_entities"),
        serde_json::json!({
            "entities": [{
                "name": "rove",
                "entityType": "project",
                "observations": ["local-first agent runtime"]
            }]
        }),
        &context,
    )
    .await;
    println!("MCP_INTEROP create_entities -> {}", created.content);
    assert!(
        created.content.contains("rove"),
        "the created entity must come back: {}",
        created.content
    );
    let created_envelope = created.envelope.as_ref().expect("an envelope is produced");
    assert_eq!(created_envelope.outcome, ToolResultOutcome::Success);
    assert_eq!(
        created_envelope
            .protocol_metadata
            .remote_tool_name
            .as_deref(),
        Some("create_entities")
    );
    assert_eq!(
        created_envelope
            .protocol_metadata
            .server_identity_hash
            .as_deref(),
        Some(snapshot.server_identity_hash.as_str())
    );
    // A text-only result is mapped into the envelope; nothing is fabricated
    // into the durable artifact store for it.
    assert!(
        created_envelope.artifacts.is_empty(),
        "a text-only result must not create a durable artifact: {:?}",
        created_envelope.artifacts
    );
    // The server declares an outputSchema, so the real structuredContent was
    // validated rather than trusted.
    let structured = created_envelope
        .structured_content
        .as_ref()
        .expect("the server returned structured content");
    assert_eq!(
        structured.schema_valid,
        Some(true),
        "structured content must satisfy the declared schema: {:?}",
        structured.schema_error
    );

    // --- Real side effect: the server wrote to the path this run confined it to
    assert!(
        memory_file.exists(),
        "the server must persist to the file this test pointed it at: {}",
        memory_file.display()
    );
    let persisted = std::fs::read_to_string(&memory_file).unwrap();
    assert!(
        persisted.contains("local-first agent runtime"),
        "the persisted graph must contain the entity this test created: {persisted}"
    );

    // --- Real call: a relation, then a read of the whole graph ---------------
    let relation = execute_checked(
        &registry,
        &format!("{namespace}create_relations"),
        serde_json::json!({
            "relations": [{
                "from": "rove",
                "to": "mcp",
                "relationType": "interoperates_with"
            }]
        }),
        &context,
    )
    .await;
    assert!(
        relation.content.contains("interoperates_with"),
        "the created relation must come back: {}",
        relation.content
    );

    let graph = execute_checked(
        &registry,
        &format!("{namespace}read_graph"),
        serde_json::json!({}),
        &context,
    )
    .await;
    println!("MCP_INTEROP read_graph -> {}", graph.content);
    for expected in ["rove", "mcp", "interoperates_with"] {
        assert!(
            graph.content.contains(expected),
            "the graph must contain {expected}: {}",
            graph.content
        );
    }

    // --- Real call: a query the server evaluates ----------------------------
    let found = execute_checked(
        &registry,
        &format!("{namespace}search_nodes"),
        serde_json::json!({ "query": "local-first" }),
        &context,
    )
    .await;
    println!("MCP_INTEROP search_nodes -> {}", found.content);
    assert!(
        found.content.contains("rove"),
        "the search must find the entity this test created: {}",
        found.content
    );

    // --- Negative: a remote refusal is a typed remote failure ----------------
    assert_remote_refusal(
        &registry,
        &format!("{namespace}add_observations"),
        serde_json::json!({
            "observations": [{
                "entityName": "no_such_entity",
                "contents": ["must not be written"]
            }]
        }),
        &context,
        "not found",
        "must not be written",
        "a remote refusal reported through isError",
    )
    .await;

    // --- Negative: an unknown tool name fails closed -------------------------
    let unknown = registry
        .execute(
            &format!("{namespace}definitely_not_a_tool"),
            serde_json::json!({}),
            &context,
        )
        .await
        .expect_err("an unknown tool name must not resolve");
    assert!(
        matches!(unknown, ToolError::UnknownTool { ref name } if name.ends_with("definitely_not_a_tool")),
        "expected a typed unknown-tool error, got {unknown:?}"
    );

    // --- Negative: the server's own schema is enforced before dispatch -------
    let missing = registry
        .execute(
            &format!("{namespace}create_entities"),
            serde_json::json!({}),
            &context,
        )
        .await
        .expect_err("a missing required argument must fail before dispatch");
    assert!(
        matches!(missing, ToolError::InvalidArgs { .. }),
        "expected a typed argument error, got {missing:?}"
    );

    let wrong_type = registry
        .execute(
            &format!("{namespace}delete_entities"),
            serde_json::json!({ "entityNames": "rove" }),
            &context,
        )
        .await
        .expect_err("a wrong argument type must fail before dispatch");
    assert!(
        matches!(wrong_type, ToolError::InvalidArgs { .. }),
        "expected a typed argument error, got {wrong_type:?}"
    );
    // That refused call was destructive: the remote graph must be untouched.
    let intact = execute_checked(
        &registry,
        &format!("{namespace}read_graph"),
        serde_json::json!({}),
        &context,
    )
    .await;
    assert!(
        intact.content.contains("rove"),
        "a refused destructive call must not mutate remote state: {}",
        intact.content
    );

    // --- Real mutation: the delete really changes remote state ---------------
    let deleted = execute_checked(
        &registry,
        &format!("{namespace}delete_entities"),
        serde_json::json!({ "entityNames": ["rove"] }),
        &context,
    )
    .await;
    assert!(
        deleted.content.to_lowercase().contains("deleted"),
        "the delete must report success: {}",
        deleted.content
    );
    let after_delete = execute_checked(
        &registry,
        &format!("{namespace}read_graph"),
        serde_json::json!({}),
        &context,
    )
    .await;
    println!(
        "MCP_INTEROP read_graph after delete -> {}",
        after_delete.content
    );
    assert!(
        !after_delete.content.contains("rove")
            && !after_delete.content.contains("interoperates_with"),
        "the deleted entity and its relation must be gone: {}",
        after_delete.content
    );

    // --- Boundedness: the third-party process tree dies with the registry ----
    drop(context);
    assert_process_tree_dies(&workspace_root.to_string_lossy(), || drop(registry)).await;
}

/// The exact protocol diagnostic for a refused official-server catalog.
///
/// Drives the same stdio client the proxy uses, so the recorded reason is the
/// real one rather than a coarse activation code. The refused server must still
/// be torn down: a refusal that leaks a process is not a bounded failure.
async fn official_filesystem_catalog_refusal(
    config: McpServerConfig,
    environment: Arc<dyn ExecutionEnvironment>,
    marker: &str,
) -> String {
    let client = rove_runtime::tools::mcp_proxy::StdioMcpClient::connect_with_environment(
        config,
        environment,
    )
    .await
    .expect("the official filesystem server must speak MCP over rove's stdio proxy");
    let reason = client
        .list_tools()
        .await
        .expect_err("a refused catalog must be reported as a protocol failure")
        .to_string();
    assert_process_tree_dies(marker, || drop(client)).await;
    reason
}

/// Records the one confirmed reason this gate cannot yet prove interop.
///
/// The assertions are deliberately narrow and drift-proof: this branch may only
/// absorb the official filesystem server's own published `read_media_file`
/// output schema, refused for the one unenforceable keyword recorded in
/// `docs/development.md`. Any other activation failure — a
/// missing `npx`, no network, a timeout, a different protocol error, a different
/// tool, or a different keyword — fails the gate. Registration succeeding means
/// the interop assertions above run instead, so a fixed subset cannot leave this
/// record quietly stale.
fn record_official_filesystem_blocker(registration: &anyhow::Error, refusal: &str) {
    let registration = registration.to_string();
    assert!(
        registration.contains("mcp_catalog_invalid"),
        "unexpected activation failure: {registration}"
    );
    assert!(
        refusal.contains("MCP output schema is invalid"),
        "unexpected catalog diagnostic: {refusal}"
    );
    assert!(
        refusal.contains(&format!(
            "tool `mcp__{OFFICIAL_FILESYSTEM_SERVER_NAME}__read_media_file`"
        )),
        "the recorded blocker must still be this tool's published output schema: {refusal}"
    );
    assert!(
        refusal.contains("unsupported keyword `anyOf`"),
        "the recorded blocker must still be this keyword: {refusal}"
    );
    // The path matters as much as the keyword: a different `anyOf` appearing
    // elsewhere in the same tool's output schema is a different blocker to
    // record, and it must not keep this record green.
    assert!(
        refusal.contains("at parameters.properties.content.items"),
        "the recorded blocker must still be at this position: {refusal}"
    );
    println!("MCP_INTEROP BLOCKED: the official server cannot be registered yet");
    println!("MCP_INTEROP activation refused: {registration}");
    println!("MCP_INTEROP protocol diagnostic: {refusal}");
    println!(
        "MCP_INTEROP this run records a blocker, NOT interoperability; see docs/development.md"
    );
}

/// Asserts a server's whole launcher chain is gone after the owner is dropped.
async fn assert_process_tree_dies(marker: &str, drop_owner: impl FnOnce()) {
    let started = processes_matching(marker).await;
    println!("MCP_INTEROP running server processes: {started:?}");
    assert!(
        !started.is_empty(),
        "the official server must be running as real processes while its client is alive"
    );

    drop_owner();

    let survivors = wait_for_processes_to_exit(&started, Duration::from_secs(30)).await;
    assert!(
        survivors.is_empty(),
        "the MCP child process tree survived the drop: {survivors:?}"
    );
    let leftovers = processes_matching(marker).await;
    assert!(
        leftovers.is_empty(),
        "processes still match the temporary root after teardown: {leftovers:?}"
    );
}

/// Executes a tool and asserts the shared envelope reports success.
async fn execute_checked(
    registry: &ToolRegistry,
    tool: &str,
    args: serde_json::Value,
    context: &ToolContext<'_>,
) -> rove_core::ToolOutput {
    let output = registry
        .execute(tool, args, context)
        .await
        .unwrap_or_else(|error| panic!("{tool} failed: {error}"));
    let envelope = output
        .envelope
        .as_ref()
        .unwrap_or_else(|| panic!("{tool} produced no envelope"));
    assert_eq!(
        envelope.outcome,
        ToolResultOutcome::Success,
        "{tool} reported {:?}: {:?}",
        envelope.outcome,
        envelope.diagnostics
    );
    output
}

/// Asserts a remote refusal is a typed failure, not a panic or a quiet success.
async fn assert_remote_refusal(
    registry: &ToolRegistry,
    tool: &str,
    args: serde_json::Value,
    context: &ToolContext<'_>,
    expected: &str,
    forbidden: &str,
    expectation: &str,
) {
    let output = match registry.execute(tool, args, context).await {
        Ok(output) => output,
        Err(error) => panic!("{expectation} must be a typed remote failure, got {error}"),
    };
    let envelope = output
        .envelope
        .as_ref()
        .unwrap_or_else(|| panic!("{expectation} produced no envelope"));
    assert_eq!(
        envelope.outcome,
        ToolResultOutcome::Error,
        "{expectation} must fail closed: {envelope:?}"
    );
    assert!(
        envelope.diagnostics.iter().any(|diagnostic| {
            diagnostic.domain == ToolErrorDomain::RemoteTool
                && diagnostic.code == "mcp_tool_reported_error"
        }),
        "{expectation} must carry the remote-tool diagnostic: {:?}",
        envelope.diagnostics
    );
    assert!(
        output
            .content
            .to_lowercase()
            .contains(&expected.to_lowercase()),
        "{expectation} must surface the server's refusal ({expected}): {}",
        output.content
    );
    assert!(
        !output.content.contains(forbidden),
        "{expectation} must not return the refused content: {}",
        output.content
    );
}

/// Every live process id whose command line mentions `marker`.
///
/// The official server is reached through a launcher chain (`cmd` -> `npx` ->
/// the server), so teardown cannot be judged from a single child id. The
/// temporary root this test created appears in every hop's command line and
/// nowhere else on the machine, which makes it an exact marker for the tree
/// this test started.
async fn processes_matching(marker: &str) -> Vec<u32> {
    let marker = marker.to_lowercase();
    process_listing()
        .await
        .lines()
        .filter(|line| line.to_lowercase().contains(&marker))
        .filter_map(|line| line.split_whitespace().next())
        .filter_map(|pid| pid.parse::<u32>().ok())
        .collect()
}

#[cfg(windows)]
async fn process_listing() -> String {
    let script =
        "Get-CimInstance Win32_Process | ForEach-Object { \"$($_.ProcessId)`t$($_.CommandLine)\" }";
    let output = timeout(
        Duration::from_secs(30),
        tokio::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("the process listing must not hang")
    .expect("the process listing must run");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[cfg(not(windows))]
async fn process_listing() -> String {
    let output = timeout(
        Duration::from_secs(30),
        tokio::process::Command::new("ps")
            .args(["-eo", "pid=,args="])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("the process listing must not hang")
    .expect("the process listing must run");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

async fn wait_for_processes_to_exit(pids: &[u32], budget: Duration) -> Vec<u32> {
    let started = Instant::now();
    loop {
        let alive: Vec<u32> = pids
            .iter()
            .copied()
            .filter(|pid| process_is_alive(*pid))
            .collect();
        if alive.is_empty() || started.elapsed() >= budget {
            return alive;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn normalize_windows_extended_path(path: std::path::PathBuf) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        let raw = path.to_string_lossy();
        if let Some(stripped) = raw.strip_prefix(r"\\?\") {
            return std::path::PathBuf::from(stripped);
        }
    }
    path
}

fn assert_process_exits(pid: u32, timeout: Duration) {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if !process_is_alive(pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("MCP child process {pid} was still alive after {timeout:?}");
}

#[cfg(windows)]
fn process_is_alive(pid: u32) -> bool {
    let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).contains(&pid.to_string())
}

#[cfg(not(windows))]
fn process_is_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
