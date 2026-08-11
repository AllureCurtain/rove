use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use rove_core::{CallId, ToolRegistry};
use rove_models::{FakeModelClient, FakeTurn};
use rove_runtime::context::ContextManager;
use rove_runtime::engine::{Engine, EngineConfig, EngineEnvironmentOptions};
use rove_runtime::environment::{
    ArtifactSink, BackgroundProcessStatus, EnvironmentError, ExecutionCapabilities,
    ExecutionEnvironment, InMemoryExecutionEnvironment, LocalExecutionEnvironment, Observation,
    ObservationStore, ProcessHost, ProcessOutput, ProcessRequest, TransientArtifactStore,
    WorkspaceFileSystem, local_environment,
};
use rove_runtime::memory::paths::MemoryPaths;
use rove_runtime::tools::fs::FsWriteTool;
use rove_runtime::tools::mcp_proxy::register_mcp_tools_from_file_with_environment;
use rove_runtime::tools::runtime_context::runtime_tool_context_with_environment;
use rove_runtime::tools::shell::{ShellPolicy, ShellTool};
use rove_runtime::types::{ApprovalDecision, ApprovalPolicy};
use rove_runtime::workspace::Workspace;
use tokio_util::sync::CancellationToken;

async fn filesystem_conformance(filesystem: &dyn WorkspaceFileSystem) {
    filesystem
        .create_utf8("src/create-only.txt", "original")
        .await
        .unwrap();
    assert!(matches!(
        filesystem
            .create_utf8("src/create-only.txt", "replacement")
            .await,
        Err(EnvironmentError::Conflict(_))
    ));
    assert_eq!(
        filesystem.read_utf8("src/create-only.txt").await.unwrap(),
        "original"
    );

    let created = filesystem
        .write_utf8("src/note.txt", "first")
        .await
        .unwrap();
    assert!(created.before.is_none());
    assert_eq!(filesystem.read_utf8("src/note.txt").await.unwrap(), "first");

    let updated = filesystem
        .write_utf8("src/note.txt", "second")
        .await
        .unwrap();
    assert_eq!(updated.before.as_deref(), Some("first"));
    assert_eq!(
        filesystem
            .list_files(Some("src"), 10)
            .await
            .unwrap()
            .into_iter()
            .map(|entry| entry.relative_path)
            .collect::<Vec<_>>(),
        vec!["src/create-only.txt", "src/note.txt"]
    );

    assert!(matches!(
        filesystem.read_utf8("../outside.txt").await,
        Err(EnvironmentError::Boundary | EnvironmentError::InvalidPath(_))
    ));
}

#[tokio::test]
async fn local_and_in_memory_filesystems_share_the_workspace_contract() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    let local = LocalExecutionEnvironment::new(&workspace);
    filesystem_conformance(local.filesystem()).await;

    let memory = InMemoryExecutionEnvironment::new(&workspace);
    filesystem_conformance(memory.filesystem()).await;
    memory.seed_file("src2/not-in-src.txt", "other").await;
    assert_eq!(
        memory
            .filesystem()
            .list_files(Some("src"), 10)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn local_filesystem_rejects_symlink_escape_when_supported() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace_root = temp.path().join("workspace");
    std::fs::create_dir(&workspace_root).unwrap();
    let outside = temp.path().join("outside.txt");
    std::fs::write(&outside, "outside").unwrap();
    let link = workspace_root.join("escape.txt");
    if !create_file_symlink(&outside, &link) {
        return;
    }
    let workspace = Workspace::detect(&workspace_root).unwrap();
    let environment = LocalExecutionEnvironment::new(&workspace);

    assert!(matches!(
        environment.filesystem().read_utf8("escape.txt").await,
        Err(EnvironmentError::Boundary | EnvironmentError::InvalidPath(_))
    ));
}

#[tokio::test]
async fn local_mutations_reject_in_workspace_symlink_targets_when_supported() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace_root = temp.path().join("workspace");
    std::fs::create_dir(&workspace_root).unwrap();
    let target = workspace_root.join("target.txt");
    std::fs::write(&target, "original").unwrap();
    let link = workspace_root.join("alias.txt");
    if !create_file_symlink(&target, &link) {
        return;
    }
    let workspace = Workspace::detect(&workspace_root).unwrap();
    let environment = LocalExecutionEnvironment::new(&workspace);

    assert_eq!(
        environment
            .filesystem()
            .read_utf8("alias.txt")
            .await
            .unwrap(),
        "original"
    );
    assert!(
        environment
            .filesystem()
            .write_utf8("alias.txt", "changed")
            .await
            .is_err()
    );
    assert!(
        environment
            .filesystem()
            .delete_path("alias.txt", false)
            .await
            .is_err()
    );
    assert!(
        environment
            .filesystem()
            .move_path("alias.txt", "moved.txt", false)
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
    assert!(link.exists());
    assert!(!workspace_root.join("moved.txt").exists());

    let target_dir = workspace_root.join("target-dir");
    std::fs::create_dir(&target_dir).unwrap();
    std::fs::write(target_dir.join("kept.txt"), "kept").unwrap();
    let dir_link = workspace_root.join("alias-dir");
    if !create_directory_symlink(&target_dir, &dir_link) {
        return;
    }
    assert!(
        environment
            .filesystem()
            .delete_path("alias-dir", true)
            .await
            .is_err()
    );
    assert!(
        environment
            .filesystem()
            .move_path("alias-dir", "moved-dir", false)
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(target_dir.join("kept.txt")).unwrap(),
        "kept"
    );
}

#[tokio::test]
async fn in_memory_process_port_enforces_bounds_timeout_cancel_and_capability_absence() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    let environment = InMemoryExecutionEnvironment::new(&workspace);
    environment
        .processes()
        .set_response(
            "fixture",
            ProcessOutput {
                status_code: Some(0),
                stdout: vec![b'x'; 128],
                stderr: vec![b'y'; 64],
                stdout_truncated: false,
                stderr_truncated: false,
            },
        )
        .await;
    let output = ProcessHost::run(
        environment.processes(),
        process_request(&workspace, "fixture", 1_000, 16),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(output.stdout.len(), 16);
    assert_eq!(output.stderr.len(), 16);
    assert!(output.stdout_truncated && output.stderr_truncated);

    environment
        .processes()
        .set_delay("fixture", Duration::from_millis(100))
        .await;
    assert!(matches!(
        ProcessHost::run(
            environment.processes(),
            process_request(&workspace, "fixture", 10, 16),
            CancellationToken::new(),
        )
        .await,
        Err(EnvironmentError::Timeout(10))
    ));
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        ProcessHost::run(
            environment.processes(),
            process_request(&workspace, "fixture", 1_000, 16),
            cancelled,
        )
        .await,
        Err(EnvironmentError::Cancelled)
    ));
    assert!(!environment.capabilities().process_stdio);
    assert!(matches!(
        ProcessHost::spawn_stdio(environment.processes(), "fixture", &[], &[]).await,
        Err(EnvironmentError::CapabilityUnavailable("process_stdio"))
    ));
}

#[tokio::test]
async fn local_process_port_bounds_output_and_cleans_up_timeout_and_cancel() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    let environment = local_environment(&workspace);
    let (program, output_args) = large_output_command();
    let output = environment
        .processes()
        .run(
            ProcessRequest {
                program,
                args: output_args,
                cwd: workspace.root.clone(),
                environment: BTreeMap::new(),
                clear_environment: false,
                timeout_ms: 5_000,
                max_output_bytes: 128,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(output.stdout.len(), 128);
    assert_eq!(output.stderr.len(), 128);
    assert!(output.stdout_truncated && output.stderr_truncated);

    let (program, args) = delayed_marker_command("timeout-marker.txt");
    assert!(matches!(
        environment
            .processes()
            .run(
                ProcessRequest {
                    program,
                    args,
                    cwd: workspace.root.clone(),
                    environment: BTreeMap::new(),
                    clear_environment: false,
                    timeout_ms: 50,
                    max_output_bytes: 128,
                },
                CancellationToken::new(),
            )
            .await,
        Err(EnvironmentError::Timeout(50))
    ));

    let (program, args) = delayed_marker_command("cancel-marker.txt");
    let cancel = CancellationToken::new();
    let cancel_for_task = cancel.clone();
    let environment_for_task = Arc::clone(&environment);
    let workspace_root = workspace.root.clone();
    let task = tokio::spawn(async move {
        environment_for_task
            .processes()
            .run(
                ProcessRequest {
                    program,
                    args,
                    cwd: workspace_root,
                    environment: BTreeMap::new(),
                    clear_environment: false,
                    timeout_ms: 5_000,
                    max_output_bytes: 128,
                },
                cancel_for_task,
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    cancel.cancel();
    assert!(matches!(
        task.await.unwrap(),
        Err(EnvironmentError::Cancelled)
    ));
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(!workspace.root.join("timeout-marker.txt").exists());
    assert!(!workspace.root.join("cancel-marker.txt").exists());
}

#[tokio::test]
async fn local_background_process_pages_progress_to_a_bounded_terminal_result() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    let environment = LocalExecutionEnvironment::new(&workspace);
    let (program, args) = large_output_command();
    let mut request = process_request(&workspace, &program, 5_000, 32);
    request.args = args;
    let started = environment
        .processes()
        .spawn_background(request, CancellationToken::new())
        .await
        .unwrap();
    assert!(!started.process_id.is_empty());

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut stdout_cursor = 0;
    let mut stderr_cursor = 0;
    let mut terminal = None;
    let mut stdout_truncated = false;
    let mut stderr_truncated = false;
    for _ in 0..250 {
        let page = environment
            .processes()
            .poll_background(&started.process_id, stdout_cursor, stderr_cursor, 8)
            .await
            .unwrap();
        stdout.extend_from_slice(&page.stdout);
        stderr.extend_from_slice(&page.stderr);
        stdout_cursor = page.stdout_cursor;
        stderr_cursor = page.stderr_cursor;
        stdout_truncated |= page.stdout_truncated;
        stderr_truncated |= page.stderr_truncated;
        if page.status != BackgroundProcessStatus::Running {
            terminal = Some(page.status);
            if page.output_complete && stdout_cursor == 32 && stderr_cursor == 32 {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert_eq!(terminal, Some(BackgroundProcessStatus::Exited));
    assert_eq!(stdout, vec![b'x'; 32]);
    assert_eq!(stderr, vec![b'y'; 32]);
    assert!(stdout_truncated);
    assert!(stderr_truncated);
    assert!(matches!(
        environment
            .processes()
            .poll_background(&started.process_id, 0, 0, 8)
            .await,
        Err(EnvironmentError::ResourceNotFound("background_process"))
    ));
}

#[tokio::test]
async fn observations_are_bounded_stable_and_version_checked() {
    let store = ObservationStore::default();
    let first = Observation::from_bytes("file:src/lib.rs", 0, b"content", "v1", false, None);
    let same = Observation::from_bytes("file:src/lib.rs", 0, b"content", "v1", false, None);
    assert_eq!(first.id, same.id);
    assert_eq!(first.byte_count, 7);
    store.put(first.clone()).await.unwrap();
    assert_eq!(store.require_version(&first.id, "v1").await.unwrap(), first);
    assert!(matches!(
        store.require_version(&first.id, "v2").await,
        Err(EnvironmentError::StaleObservation)
    ));

    for index in 1..512 {
        store
            .put(Observation::from_bytes(
                format!("source:{index}"),
                0,
                b"x",
                "v1",
                false,
                None,
            ))
            .await
            .unwrap();
    }
    assert!(matches!(
        store
            .put(Observation::from_bytes(
                "source:overflow",
                0,
                b"x",
                "v1",
                false,
                None,
            ))
            .await,
        Err(EnvironmentError::ResourceLimit("observations"))
    ));
}

#[tokio::test]
async fn transient_artifact_projections_are_item_bounded() {
    let store = TransientArtifactStore::default();
    for index in 0..512 {
        assert!(
            store
                .put(&format!("source:{index}"), b"x")
                .await
                .unwrap()
                .is_some()
        );
    }
    assert!(matches!(
        store.put("source:overflow", b"x").await,
        Err(EnvironmentError::ResourceLimit("artifact_projections"))
    ));
}

#[tokio::test]
async fn invocation_services_accept_an_explicit_in_memory_environment() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    let environment = Arc::new(InMemoryExecutionEnvironment::new(&workspace));
    let context = runtime_tool_context_with_environment(
        CallId::new(),
        &workspace,
        MemoryPaths::from_workspace(&workspace, 8),
        ApprovalPolicy::Auto,
        None,
        CancellationToken::new(),
        environment.clone(),
    );
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(FsWriteTool::new(workspace.root.clone())));
    registry
        .execute(
            "write_file",
            serde_json::json!({"path": "virtual.txt", "content": "memory only"}),
            &context,
        )
        .await
        .unwrap();

    assert_eq!(
        ExecutionEnvironment::filesystem(environment.as_ref())
            .read_utf8("virtual.txt")
            .await
            .unwrap(),
        "memory only"
    );
    assert!(!workspace.root.join("virtual.txt").exists());
    let serialized = serde_json::to_string(environment.identity()).unwrap();
    assert!(!serialized.contains(&workspace.root.to_string_lossy().to_string()));
    assert!(
        environment
            .identity()
            .workspace_digest
            .starts_with("sha256:")
    );
    assert_eq!(
        environment.identity().workspace_digest,
        rove_runtime::runtime_identity::workspace_fingerprint(&workspace)
    );
}

#[tokio::test]
async fn engine_reuses_one_injected_environment_for_file_and_shell_tools() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    let environment = Arc::new(InMemoryExecutionEnvironment::new(&workspace));
    let shell_program = if cfg!(windows) { "powershell" } else { "sh" };
    environment
        .processes()
        .set_response(
            shell_program,
            ProcessOutput {
                status_code: Some(0),
                stdout: b"virtual-shell-output".to_vec(),
                stderr: Vec::new(),
                stdout_truncated: false,
                stderr_truncated: false,
            },
        )
        .await;
    let injected: Arc<dyn ExecutionEnvironment> = environment.clone();
    let model = FakeModelClient::with_turns(
        "unused".to_string(),
        vec![
            FakeTurn::ToolUse {
                id: "write-memory".to_string(),
                name: "write_file".to_string(),
                args: serde_json::json!({
                    "path": "engine-virtual.txt",
                    "content": "memory only"
                }),
            },
            FakeTurn::ToolUse {
                id: "shell-memory".to_string(),
                name: "run_shell".to_string(),
                args: serde_json::json!({"command": "virtual-command"}),
            },
            FakeTurn::Text("done".to_string()),
        ],
    );
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(FsWriteTool::new(workspace.root.clone())));
    registry.register(Box::new(ShellTool::with_policy(
        workspace.root.clone(),
        ShellPolicy::default(),
    )));
    let engine = Engine::with_workspace_and_approval_decision_and_environment(
        Box::new(model),
        registry,
        ContextManager::new("test system prompt".to_string()),
        EngineConfig::new(4, false),
        workspace.clone(),
        EngineEnvironmentOptions {
            approval_policy: ApprovalPolicy::Auto,
            approval_decision: ApprovalDecision::Approve,
            environment: injected.clone(),
        },
    );
    let runtime_identity = engine.runtime_identity();
    assert_eq!(
        runtime_identity
            .execution_environment
            .as_ref()
            .map(|identity| identity.adapter.as_str()),
        Some("in_memory")
    );
    assert_eq!(
        runtime_identity.execution_capabilities,
        Some(*injected.capabilities())
    );

    let events = engine
        .ask("exercise injected environment".to_string(), None)
        .collect::<Vec<_>>()
        .await;

    assert!(Arc::ptr_eq(engine.execution_environment(), &injected));
    assert_eq!(
        environment
            .filesystem()
            .read_utf8("engine-virtual.txt")
            .await
            .unwrap(),
        "memory only"
    );
    assert!(!workspace.root.join("engine-virtual.txt").exists());
    assert!(events.iter().any(|event| matches!(
        event,
        rove_runtime::events::StreamEvent::ToolCallCompleted { result, .. }
            if result.output.contains("virtual-shell-output")
    )));
}

#[tokio::test]
async fn mcp_configuration_reads_through_the_in_memory_filesystem_port() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    let environment = Arc::new(InMemoryExecutionEnvironment::new(&workspace));
    environment
        .seed_file(".rove/mcp_servers.json", r#"{"servers":[]}"#)
        .await;
    let mut registry = ToolRegistry::new();

    let registered = register_mcp_tools_from_file_with_environment(
        &mut registry,
        workspace.root.join(".rove/mcp_servers.json"),
        environment,
    )
    .await
    .unwrap();

    assert_eq!(registered, 0);
    assert!(!workspace.root.join(".rove/mcp_servers.json").exists());
}

#[tokio::test]
async fn missing_invocation_capability_is_rejected_before_a_side_effect() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    let environment = Arc::new(InMemoryExecutionEnvironment::with_capabilities(
        &workspace,
        ExecutionCapabilities::default(),
    ));
    let context = runtime_tool_context_with_environment(
        CallId::new(),
        &workspace,
        MemoryPaths::from_workspace(&workspace, 8),
        ApprovalPolicy::Auto,
        None,
        CancellationToken::new(),
        environment.clone(),
    );
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(FsWriteTool::new(workspace.root.clone())));

    let error = registry
        .execute(
            "write_file",
            serde_json::json!({"path": "blocked.txt", "content": "must not write"}),
            &context,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        rove_core::ToolError::PermissionDenied { .. }
    ));
    assert!(
        ExecutionEnvironment::filesystem(environment.as_ref())
            .read_utf8("blocked.txt")
            .await
            .is_err()
    );
}

fn process_request(
    workspace: &Workspace,
    program: &str,
    timeout_ms: u64,
    max_output_bytes: usize,
) -> ProcessRequest {
    ProcessRequest {
        program: program.to_string(),
        args: Vec::new(),
        cwd: workspace.root.clone(),
        environment: BTreeMap::new(),
        clear_environment: false,
        timeout_ms,
        max_output_bytes,
    }
}

#[cfg(windows)]
fn large_output_command() -> (String, Vec<String>) {
    (
        "powershell".to_string(),
        vec![
            "-NoProfile".to_string(),
            "-NonInteractive".to_string(),
            "-Command".to_string(),
            "[Console]::Out.Write('x' * 10000); [Console]::Error.Write('y' * 10000)".to_string(),
        ],
    )
}

#[cfg(not(windows))]
fn large_output_command() -> (String, Vec<String>) {
    (
        "sh".to_string(),
        vec![
            "-lc".to_string(),
            "head -c 10000 /dev/zero | tr '\\0' x; head -c 10000 /dev/zero | tr '\\0' y >&2"
                .to_string(),
        ],
    )
}

#[cfg(windows)]
fn delayed_marker_command(marker: &str) -> (String, Vec<String>) {
    (
        "powershell".to_string(),
        vec![
            "-NoProfile".to_string(),
            "-NonInteractive".to_string(),
            "-Command".to_string(),
            format!("Start-Sleep -Milliseconds 400; Set-Content -LiteralPath '{marker}' x"),
        ],
    )
}

#[cfg(not(windows))]
fn delayed_marker_command(marker: &str) -> (String, Vec<String>) {
    (
        "sh".to_string(),
        vec!["-lc".to_string(), format!("sleep 0.4; touch '{marker}'")],
    )
}

/// `create_utf8` must not report a mutation before the bytes are readable.
///
/// A `tokio::fs::File` write is completed by an in-flight blocking operation, so
/// dropping the handle without flushing leaves the bytes landing at an
/// unspecified later moment. The read here deliberately uses `std::fs`, which
/// does not queue behind that operation the way `filesystem().read_utf8()` does,
/// so it observes the file the way an outside caller would.
///
/// This locks the contract; it does not by itself reproduce the original
/// load-dependent race, which needed a saturated blocking pool to surface.
#[test]
fn create_utf8_bytes_are_durable_before_it_reports_success() {
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path().to_path_buf();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();

    runtime.block_on(async {
        let workspace = Workspace::detect(&root).unwrap();
        let environment = local_environment(&workspace);
        let filesystem = environment.filesystem();

        for index in 0..24 {
            let relative = format!("durable/note-{index}.txt");
            let content = format!("content-{index}");
            filesystem.create_utf8(&relative, &content).await.unwrap();
            assert_eq!(
                std::fs::read_to_string(root.join("durable").join(format!("note-{index}.txt")))
                    .unwrap(),
                content,
                "create_utf8 returned before {relative} held its bytes"
            );
        }
    });
}

#[cfg(unix)]
fn create_file_symlink(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn create_file_symlink(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_file(target, link).is_ok()
}

#[cfg(unix)]
fn create_directory_symlink(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn create_directory_symlink(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_dir(target, link).is_ok()
}
