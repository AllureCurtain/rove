use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use super::mcp_proxy::{McpConfigFile, McpServerConfig, McpTransport};

pub const MAX_PRODUCT_MCP_SERVERS: usize = 32;
pub const MIN_PRODUCT_MCP_TIMEOUT_MS: u64 = 100;
pub const MAX_PRODUCT_MCP_TIMEOUT_MS: u64 = 120_000;

const MAX_CONFIG_BYTES: u64 = 256 * 1024;
const MAX_SERVER_NAME_BYTES: usize = 64;
const MAX_COMMAND_BYTES: usize = 2_048;
const MAX_URL_BYTES: usize = 2_048;
const MAX_ARGUMENTS: usize = 64;
const MAX_ARGUMENT_BYTES: usize = 2_048;
const MAX_ENV_NAMES: usize = 32;
const MAX_STDERR_CAPTURE_BYTES: usize = 64 * 1024;
const LOCK_STALE_AFTER: Duration = Duration::from_secs(30);
const LOCK_FILE_NAME: &str = ".mcp_servers.lock";
const TEMP_FILE_NAME: &str = ".mcp_servers.json.tmp";
const BACKUP_FILE_NAME: &str = ".mcp_servers.json.bak";
const READY_FILE_NAME: &str = ".mcp_servers.json.ready";
const READY_MARKER: &[u8] = b"rove-mcp-config-replacement-v1\n";
static MCP_CONFIG_LOCK: Mutex<()> = Mutex::new(());

pub fn list_product_mcp_servers_sync(path: &Path) -> std::io::Result<Vec<McpServerConfig>> {
    // A read of an unmaterialized catalog must stay side-effect free. Once
    // the parent exists we still take the lock so interrupted replacements
    // can be recovered before reading.
    if !config_parent_is_present(path)? {
        return Ok(Vec::new());
    }
    with_config_lock(path, read_config_unlocked)
}

pub fn create_product_mcp_server_sync(
    path: &Path,
    server: McpServerConfig,
) -> std::io::Result<McpServerConfig> {
    validate_product_mcp_server(&server)?;
    with_config_lock(path, |path| {
        let mut servers = read_config_unlocked(path)?;
        if servers.iter().any(|current| current.name == server.name) {
            return Err(std::io::Error::new(
                ErrorKind::AlreadyExists,
                "MCP server already exists",
            ));
        }
        if servers.len() >= MAX_PRODUCT_MCP_SERVERS {
            return Err(invalid_input("too many MCP servers"));
        }
        servers.push(server.clone());
        sort_servers(&mut servers);
        write_config_unlocked(path, &servers)?;
        Ok(server)
    })
}

pub fn update_product_mcp_server_sync(
    path: &Path,
    name: &str,
    server: McpServerConfig,
) -> std::io::Result<McpServerConfig> {
    if name != server.name {
        return Err(invalid_input("MCP server name is immutable"));
    }
    validate_product_mcp_server(&server)?;
    with_config_lock(path, |path| {
        let mut servers = read_config_unlocked(path)?;
        let Some(index) = servers.iter().position(|current| current.name == name) else {
            return Err(std::io::Error::new(
                ErrorKind::NotFound,
                "MCP server does not exist",
            ));
        };
        servers[index] = server.clone();
        sort_servers(&mut servers);
        write_config_unlocked(path, &servers)?;
        Ok(server)
    })
}

pub fn delete_product_mcp_server_sync(path: &Path, name: &str) -> std::io::Result<()> {
    validate_product_mcp_server_name(name)?;
    with_config_lock(path, |path| {
        let mut servers = read_config_unlocked(path)?;
        let original_len = servers.len();
        servers.retain(|server| server.name != name);
        if servers.len() == original_len {
            return Err(std::io::Error::new(
                ErrorKind::NotFound,
                "MCP server does not exist",
            ));
        }
        write_config_unlocked(path, &servers)
    })
}

/// Seed a new authoritative Product Settings catalog from the currently
/// effective legacy catalog. The target lock makes first-write promotion
/// race-safe; an already-materialized target always wins and is validated.
pub fn promote_product_mcp_catalog_sync(source: &Path, target: &Path) -> std::io::Result<()> {
    if source == target {
        return Ok(());
    }
    with_config_lock(target, |target| {
        if target.exists() {
            read_config_unlocked(target)?;
            return Ok(());
        }
        let servers = read_config_unlocked(source)?;
        write_config_unlocked(target, &servers)
    })
}

pub fn validate_product_mcp_server_name(name: &str) -> std::io::Result<()> {
    if !is_valid_server_name(name) {
        return Err(invalid_input("invalid MCP server name"));
    }
    Ok(())
}

pub fn validate_product_mcp_server(server: &McpServerConfig) -> std::io::Result<()> {
    if !is_valid_server_name(&server.name) {
        return Err(invalid_input("invalid MCP server name"));
    }
    if !server.env.is_empty() {
        return Err(invalid_input(
            "raw MCP environment values are not accepted by product settings",
        ));
    }
    if server.policy.request_timeout_ms < MIN_PRODUCT_MCP_TIMEOUT_MS
        || server.policy.request_timeout_ms > MAX_PRODUCT_MCP_TIMEOUT_MS
        || server.policy.stderr_capture_bytes > MAX_STDERR_CAPTURE_BYTES
    {
        return Err(invalid_input("invalid MCP transport policy"));
    }
    if server.args.len() > MAX_ARGUMENTS
        || server.args.iter().any(|argument| {
            !valid_text(argument, MAX_ARGUMENT_BYTES) || looks_like_secret(argument)
        })
    {
        return Err(invalid_input("invalid or secret-shaped MCP argument"));
    }
    if server.env_names.len() > MAX_ENV_NAMES {
        return Err(invalid_input("too many MCP environment names"));
    }
    let mut env_names = HashSet::new();
    if server
        .env_names
        .iter()
        .any(|name| !is_valid_environment_name(name) || !env_names.insert(name))
    {
        return Err(invalid_input("invalid or duplicate MCP environment name"));
    }
    match server.transport {
        McpTransport::Stdio => {
            if !valid_nonempty_text(&server.command, MAX_COMMAND_BYTES) || !server.url.is_empty() {
                return Err(invalid_input("invalid stdio MCP configuration"));
            }
        }
        // Both HTTP transports validate their URL identically. They stay
        // separate variants so diagnostics can mark legacy SSE deprecated and
        // neither one inherits the other's feature claims.
        McpTransport::Sse | McpTransport::StreamableHttp => {
            if !server.command.is_empty() || !server.args.is_empty() || !server.env_names.is_empty()
            {
                return Err(invalid_input("invalid HTTP MCP configuration"));
            }
            let url =
                reqwest::Url::parse(&server.url).map_err(|_| invalid_input("invalid MCP URL"))?;
            if server.url.len() > MAX_URL_BYTES
                || !matches!(url.scheme(), "http" | "https")
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
            {
                return Err(invalid_input("invalid MCP URL"));
            }
        }
    }
    Ok(())
}

fn with_config_lock<T>(
    path: &Path,
    operation: impl FnOnce(&Path) -> std::io::Result<T>,
) -> std::io::Result<T> {
    let _process_guard = process_guard()?;
    let parent = prepare_parent(path)?;
    let _file_guard = ConfigFileLock::acquire(&parent)?;
    recover_interrupted_replacement(path)?;
    operation(path)
}

fn config_parent_is_present(path: &Path) -> std::io::Result<bool> {
    if path.file_name().and_then(|name| name.to_str()) != Some("mcp_servers.json") {
        return Err(invalid_input("unexpected MCP config filename"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid_input("MCP config has no parent directory"))?;
    match fs::symlink_metadata(parent) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(
            permission_denied("MCP config parent must be a regular directory"),
        ),
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn process_guard() -> std::io::Result<MutexGuard<'static, ()>> {
    MCP_CONFIG_LOCK
        .lock()
        .map_err(|_| std::io::Error::other("MCP config lock was poisoned"))
}

fn prepare_parent(path: &Path) -> std::io::Result<PathBuf> {
    if path.file_name().and_then(|name| name.to_str()) != Some("mcp_servers.json") {
        return Err(invalid_input("unexpected MCP config filename"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid_input("MCP config has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let metadata = fs::symlink_metadata(parent)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(permission_denied(
            "MCP config parent must be a regular directory",
        ));
    }
    Ok(parent.to_path_buf())
}

struct ConfigFileLock {
    path: PathBuf,
}

impl ConfigFileLock {
    fn acquire(parent: &Path) -> std::io::Result<Self> {
        let path = parent.join(LOCK_FILE_NAME);
        for attempt in 0..2 {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    let guard = Self { path: path.clone() };
                    writeln!(file, "{}", std::process::id())?;
                    file.sync_all()?;
                    return Ok(guard);
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists && attempt == 0 => {
                    let metadata = fs::symlink_metadata(&path)?;
                    if metadata.file_type().is_symlink() || !metadata.is_file() {
                        return Err(permission_denied("unsafe MCP config lock file"));
                    }
                    let stale = metadata
                        .modified()
                        .ok()
                        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                        .is_some_and(|age| age >= LOCK_STALE_AFTER);
                    if stale {
                        fs::remove_file(&path)?;
                        continue;
                    }
                    return Err(std::io::Error::new(
                        ErrorKind::WouldBlock,
                        "MCP config is locked by another process",
                    ));
                }
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            ErrorKind::WouldBlock,
            "MCP config lock could not be acquired",
        ))
    }
}

impl Drop for ConfigFileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn read_config_unlocked(path: &Path) -> std::io::Result<Vec<McpServerConfig>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    validate_regular_file(path)?;
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_CONFIG_BYTES {
        return Err(invalid_data("MCP config exceeds the supported size"));
    }
    let file = File::open(path)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_CONFIG_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(invalid_data("MCP config exceeds the supported size"));
    }
    let config: McpConfigFile =
        serde_json::from_slice(&bytes).map_err(|_| invalid_data("MCP config is invalid JSON"))?;
    if config.servers.len() > MAX_PRODUCT_MCP_SERVERS {
        return Err(invalid_data("MCP config contains too many servers"));
    }
    let mut names = HashSet::new();
    for server in &config.servers {
        validate_product_mcp_server(server)
            .map_err(|_| invalid_data("MCP config contains an invalid server"))?;
        if !names.insert(&server.name) {
            return Err(invalid_data("MCP config contains duplicate server names"));
        }
    }
    let mut servers = config.servers;
    sort_servers(&mut servers);
    Ok(servers)
}

fn write_config_unlocked(path: &Path, servers: &[McpServerConfig]) -> std::io::Result<()> {
    let parent = path.parent().expect("validated MCP config parent");
    let temporary = parent.join(TEMP_FILE_NAME);
    let backup = parent.join(BACKUP_FILE_NAME);
    let ready = parent.join(READY_FILE_NAME);
    ensure_absent(&temporary)?;
    ensure_absent(&backup)?;
    ensure_absent(&ready)?;

    let mut bytes = serde_json::to_vec_pretty(&McpConfigFile {
        servers: servers.to_vec(),
    })
    .map_err(|_| invalid_data("MCP config could not be serialized"))?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(invalid_input("MCP config exceeds the supported size"));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    let mut marker = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&ready)?;
    marker.write_all(READY_MARKER)?;
    marker.sync_all()?;

    if path.exists() {
        validate_regular_file(path)?;
        fs::rename(path, &backup)?;
    }
    fs::rename(&temporary, path)?;
    sync_directory(parent)?;
    if backup.exists() {
        fs::remove_file(&backup)?;
    }
    fs::remove_file(&ready)?;
    sync_directory(parent)?;
    Ok(())
}

fn recover_interrupted_replacement(path: &Path) -> std::io::Result<()> {
    let parent = path.parent().expect("validated MCP config parent");
    let temporary = parent.join(TEMP_FILE_NAME);
    let backup = parent.join(BACKUP_FILE_NAME);
    let ready = parent.join(READY_FILE_NAME);
    for candidate in [&temporary, &backup, &ready] {
        if candidate.exists() {
            validate_regular_file(candidate)?;
        }
    }
    let has_ready = ready.exists();
    if has_ready && fs::read(&ready)? != READY_MARKER {
        return Err(invalid_data("MCP config recovery marker is invalid"));
    }

    if has_ready {
        match (path.exists(), backup.exists(), temporary.exists()) {
            (true, true, false) => {
                read_config_unlocked(path)?;
                fs::remove_file(&backup)?;
            }
            (true, false, true) => {
                read_config_unlocked(path)?;
                fs::remove_file(&temporary)?;
            }
            (true, false, false) => {
                read_config_unlocked(path)?;
            }
            (false, true, _) => {
                if temporary.exists() {
                    fs::remove_file(&temporary)?;
                }
                fs::rename(&backup, path)?;
                read_config_unlocked(path)?;
            }
            (false, false, true) => {
                fs::remove_file(&temporary)?;
            }
            (false, false, false) => {}
            (true, true, true) => {
                return Err(invalid_data("MCP config recovery state is ambiguous"));
            }
        }
        fs::remove_file(&ready)?;
    } else {
        if temporary.exists() {
            fs::remove_file(&temporary)?;
        }
        match (path.exists(), backup.exists()) {
            (false, true) => fs::rename(&backup, path)?,
            (true, true) => {
                read_config_unlocked(path)?;
                fs::remove_file(&backup)?;
            }
            _ => {}
        }
    }
    sync_directory(parent)?;
    Ok(())
}

fn validate_regular_file(path: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(permission_denied(
            "MCP config artifact must be a regular file",
        ));
    }
    Ok(())
}

fn ensure_absent(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        return Err(std::io::Error::new(
            ErrorKind::AlreadyExists,
            "MCP config recovery artifact already exists",
        ));
    }
    Ok(())
}

fn sort_servers(servers: &mut [McpServerConfig]) {
    servers.sort_by(|left, right| left.name.cmp(&right.name));
}

fn is_valid_server_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_SERVER_NAME_BYTES
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
}

fn is_valid_environment_name(name: &str) -> bool {
    let mut characters = name.chars();
    matches!(characters.next(), Some(first) if first == '_' || first.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn valid_nonempty_text(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty() && valid_text(value, max_bytes)
}

fn valid_text(value: &str, max_bytes: usize) -> bool {
    value.len() <= max_bytes && !value.chars().any(char::is_control)
}

fn looks_like_secret(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "sk-",
        "bearer ",
        "api_key=",
        "api-key=",
        "token=",
        "password=",
        "secret=",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn invalid_input(message: &'static str) -> std::io::Error {
    std::io::Error::new(ErrorKind::InvalidInput, message)
}

fn invalid_data(message: &'static str) -> std::io::Error {
    std::io::Error::new(ErrorKind::InvalidData, message)
}

fn permission_denied(message: &'static str) -> std::io::Error {
    std::io::Error::new(ErrorKind::PermissionDenied, message)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tempfile::TempDir;

    use super::*;
    use crate::tools::mcp_proxy::McpTransportPolicy;

    fn config_path(temp: &TempDir) -> PathBuf {
        temp.path().join(".rove/mcp_servers.json")
    }

    fn stdio_server(name: &str) -> McpServerConfig {
        McpServerConfig {
            name: name.to_string(),
            enabled: true,
            required: true,
            transport: McpTransport::Stdio,
            command: "python".to_string(),
            args: vec!["server.py".to_string()],
            env: HashMap::new(),
            env_names: vec!["PATH".to_string()],
            url: String::new(),
            policy: McpTransportPolicy {
                request_timeout_ms: 5_000,
                stderr_capture_bytes: 16 * 1024,
            },
        }
    }

    #[test]
    fn product_config_crud_is_atomic_and_never_persists_raw_environment_values() {
        let temp = TempDir::new().unwrap();
        let path = config_path(&temp);
        let created = create_product_mcp_server_sync(&path, stdio_server("filesystem")).unwrap();
        assert_eq!(created.env_names, ["PATH"]);
        let stored = fs::read_to_string(&path).unwrap();
        assert!(stored.contains("\"env_names\""));
        assert!(!stored.contains(std::env::var("PATH").as_deref().unwrap_or("missing")));

        let duplicate =
            create_product_mcp_server_sync(&path, stdio_server("filesystem")).unwrap_err();
        assert_eq!(duplicate.kind(), ErrorKind::AlreadyExists);

        let mut updated = stdio_server("filesystem");
        updated.enabled = false;
        updated.policy.request_timeout_ms = 9_000;
        update_product_mcp_server_sync(&path, "filesystem", updated.clone()).unwrap();
        assert_eq!(list_product_mcp_servers_sync(&path).unwrap(), [updated]);
        delete_product_mcp_server_sync(&path, "filesystem").unwrap();
        assert!(list_product_mcp_servers_sync(&path).unwrap().is_empty());
        assert_eq!(
            delete_product_mcp_server_sync(&path, "filesystem")
                .unwrap_err()
                .kind(),
            ErrorKind::NotFound
        );
    }

    #[test]
    fn missing_product_config_read_does_not_create_its_parent() {
        let temp = TempDir::new().unwrap();
        let path = config_path(&temp);

        assert!(list_product_mcp_servers_sync(&path).unwrap().is_empty());
        assert!(!path.parent().unwrap().exists());
    }

    #[test]
    fn product_config_rejects_raw_or_secret_shaped_values() {
        let mut server = stdio_server("unsafe");
        server
            .env
            .insert("TOKEN".to_string(), "raw-secret".to_string());
        assert_eq!(
            validate_product_mcp_server(&server).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
        server.env.clear();
        server.args.push("--token=sk-secret-canary".to_string());
        assert_eq!(
            validate_product_mcp_server(&server).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
    }

    #[test]
    fn product_config_recovers_backup_and_committed_replacement_states() {
        let temp = TempDir::new().unwrap();
        let path = config_path(&temp);
        create_product_mcp_server_sync(&path, stdio_server("original")).unwrap();
        let parent = path.parent().unwrap();
        let backup = parent.join(BACKUP_FILE_NAME);
        let temporary = parent.join(TEMP_FILE_NAME);
        let ready = parent.join(READY_FILE_NAME);

        fs::rename(&path, &backup).unwrap();
        fs::write(&temporary, b"uncommitted").unwrap();
        fs::write(&ready, READY_MARKER).unwrap();
        assert_eq!(
            list_product_mcp_servers_sync(&path).unwrap()[0].name,
            "original"
        );
        assert!(!backup.exists() && !temporary.exists() && !ready.exists());

        fs::copy(&path, &backup).unwrap();
        let committed = McpConfigFile {
            servers: vec![stdio_server("committed")],
        };
        fs::write(&path, serde_json::to_vec_pretty(&committed).unwrap()).unwrap();
        fs::write(&ready, READY_MARKER).unwrap();
        assert_eq!(
            list_product_mcp_servers_sync(&path).unwrap()[0].name,
            "committed"
        );
        assert!(!backup.exists() && !ready.exists());
    }

    #[test]
    fn product_config_promotes_legacy_once_without_overwriting_the_target() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("legacy/mcp_servers.json");
        let target = temp.path().join("contract/mcp_servers.json");
        create_product_mcp_server_sync(&source, stdio_server("legacy")).unwrap();

        promote_product_mcp_catalog_sync(&source, &target).unwrap();
        assert_eq!(
            list_product_mcp_servers_sync(&target).unwrap()[0].name,
            "legacy"
        );

        create_product_mcp_server_sync(&target, stdio_server("contract")).unwrap();
        create_product_mcp_server_sync(&source, stdio_server("late_legacy")).unwrap();
        promote_product_mcp_catalog_sync(&source, &target).unwrap();
        let names = list_product_mcp_servers_sync(&target)
            .unwrap()
            .into_iter()
            .map(|server| server.name)
            .collect::<Vec<_>>();
        assert_eq!(names, ["contract", "legacy"]);
    }

    #[test]
    fn product_config_rejects_a_recent_external_lock_and_unsafe_json() {
        let temp = TempDir::new().unwrap();
        let path = config_path(&temp);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path.parent().unwrap().join(LOCK_FILE_NAME), "other").unwrap();
        assert_eq!(
            list_product_mcp_servers_sync(&path).unwrap_err().kind(),
            ErrorKind::WouldBlock
        );
        fs::remove_file(path.parent().unwrap().join(LOCK_FILE_NAME)).unwrap();
        fs::write(
            &path,
            br#"{"servers":[{"name":"unsafe","env":{"TOKEN":"raw"}}]}"#,
        )
        .unwrap();
        assert_eq!(
            list_product_mcp_servers_sync(&path).unwrap_err().kind(),
            ErrorKind::InvalidData
        );
    }
}
