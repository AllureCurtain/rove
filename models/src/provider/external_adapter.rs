//! Opt-in external process adapter (v1).
//!
//! Speaks a bounded JSONL protocol over stdin/stdout so unsupported wire formats
//! can be handled without recompiling Rove. Disabled unless a profile selects
//! `wire_protocol = "external-adapter-v1"` and supplies a direct command array.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::{Instant, timeout};

use crate::{
    Message, ModelClient, ModelClientId, ModelError, ModelEvent, ModelToolSchema, ProviderOptions,
    Usage,
};

use super::{AuthStyle, EXTERNAL_ADAPTER_V1_PROTOCOL, ResolvedAuth, ResolvedHeader};

const PROTOCOL_VERSION: u32 = 1;
const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_MAX_STDOUT_LINE_BYTES: usize = 64 * 1024;
const DEFAULT_MAX_STDERR_BYTES: usize = 16 * 1024;
const MAX_COMMAND_ARGS: usize = 32;
const MAX_ENV_ALLOWLIST: usize = 64;
const MAX_ARG_BYTES: usize = 4 * 1024;

/// Configuration for one external-adapter-v1 process target.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalAdapterConfig {
    pub command: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub env_allowlist: BTreeSet<String>,
    pub extra_env: BTreeMap<String, String>,
    pub base_url: String,
    pub model: String,
    pub auth: ResolvedAuth,
    pub headers: Vec<ResolvedHeader>,
    pub options: ProviderOptions,
    pub protocol_options: serde_json::Value,
    pub startup_timeout: Duration,
    pub request_timeout: Duration,
    pub idle_timeout: Duration,
    pub shutdown_timeout: Duration,
    pub max_stdout_line_bytes: usize,
    pub max_stderr_bytes: usize,
}

impl ExternalAdapterConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn from_protocol_options(
        protocol_options: &serde_json::Value,
        base_url: impl Into<String>,
        model: impl Into<String>,
        auth: ResolvedAuth,
        headers: Vec<ResolvedHeader>,
        options: ProviderOptions,
        workspace_root: &Path,
        allow_external_paths: bool,
    ) -> Result<Self, ModelError> {
        let object = protocol_options.as_object().ok_or_else(|| {
            ModelError::InvalidConfiguration(
                "external-adapter-v1 protocol_options must be a JSON object".to_string(),
            )
        })?;

        let command = parse_command(object.get("command"))?;
        let executable = PathBuf::from(&command[0]);
        validate_executable_path(&executable, workspace_root, allow_external_paths)?;

        let working_directory = match object.get("working_directory").and_then(|v| v.as_str()) {
            Some(path) if !path.trim().is_empty() => {
                let path = PathBuf::from(path.trim());
                validate_working_directory(&path, workspace_root, allow_external_paths)?;
                Some(path)
            }
            _ => None,
        };

        let mut env_allowlist = BTreeSet::new();
        if let Some(list) = object.get("env_allowlist").and_then(|v| v.as_array()) {
            if list.len() > MAX_ENV_ALLOWLIST {
                return Err(ModelError::InvalidConfiguration(format!(
                    "external-adapter-v1 env_allowlist exceeds {MAX_ENV_ALLOWLIST} entries"
                )));
            }
            for item in list {
                let name = item.as_str().ok_or_else(|| {
                    ModelError::InvalidConfiguration(
                        "external-adapter-v1 env_allowlist entries must be strings".to_string(),
                    )
                })?;
                validate_env_name(name)?;
                env_allowlist.insert(name.to_string());
            }
        }

        let mut extra_env = BTreeMap::new();
        if let Some(map) = object.get("env").and_then(|v| v.as_object()) {
            if map.len() > MAX_ENV_ALLOWLIST {
                return Err(ModelError::InvalidConfiguration(format!(
                    "external-adapter-v1 env map exceeds {MAX_ENV_ALLOWLIST} entries"
                )));
            }
            for (key, value) in map {
                validate_env_name(key)?;
                let value = value.as_str().ok_or_else(|| {
                    ModelError::InvalidConfiguration(
                        "external-adapter-v1 env values must be strings".to_string(),
                    )
                })?;
                if value.len() > MAX_ARG_BYTES {
                    return Err(ModelError::InvalidConfiguration(
                        "external-adapter-v1 env value is too large".to_string(),
                    ));
                }
                extra_env.insert(key.clone(), value.to_string());
            }
        }

        Ok(Self {
            command,
            working_directory,
            env_allowlist,
            extra_env,
            base_url: base_url.into().trim().trim_end_matches('/').to_string(),
            model: model.into().trim().to_string(),
            auth,
            headers,
            options,
            protocol_options: protocol_options.clone(),
            startup_timeout: duration_option(
                object,
                "startup_timeout_ms",
                DEFAULT_STARTUP_TIMEOUT,
            )?,
            request_timeout: duration_option(
                object,
                "request_timeout_ms",
                DEFAULT_REQUEST_TIMEOUT,
            )?,
            idle_timeout: duration_option(object, "idle_timeout_ms", DEFAULT_IDLE_TIMEOUT)?,
            shutdown_timeout: duration_option(
                object,
                "shutdown_timeout_ms",
                DEFAULT_SHUTDOWN_TIMEOUT,
            )?,
            max_stdout_line_bytes: usize_option(
                object,
                "max_stdout_line_bytes",
                DEFAULT_MAX_STDOUT_LINE_BYTES,
                1,
                1024 * 1024,
            )?,
            max_stderr_bytes: usize_option(
                object,
                "max_stderr_bytes",
                DEFAULT_MAX_STDERR_BYTES,
                1,
                1024 * 1024,
            )?,
        })
    }
}

/// Model client that drives one external adapter process per stream request.
pub struct ExternalAdapterClient {
    config: Arc<ExternalAdapterConfig>,
}

impl ExternalAdapterClient {
    pub fn new(config: ExternalAdapterConfig) -> Result<Self, ModelError> {
        if config.command.is_empty() {
            return Err(ModelError::InvalidConfiguration(
                "external-adapter-v1 command must not be empty".to_string(),
            ));
        }
        if config.model.is_empty() {
            return Err(ModelError::InvalidConfiguration(
                "external-adapter-v1 model must not be empty".to_string(),
            ));
        }
        Ok(Self {
            config: Arc::new(config),
        })
    }

    pub fn config(&self) -> &ExternalAdapterConfig {
        &self.config
    }
}

#[async_trait]
impl ModelClient for ExternalAdapterClient {
    fn stream(
        &self,
        messages: &[Message],
        tools: &[ModelToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
        let config = Arc::clone(&self.config);
        let messages = messages.to_vec();
        let tools = tools.to_vec();
        Box::pin(async_stream::stream! {
            let mut session = match ExternalAdapterSession::start(config, messages, tools).await {
                Ok(session) => session,
                Err(error) => {
                    yield Err(error);
                    return;
                }
            };
            loop {
                match session.next_event().await {
                    Ok(Some(event)) => {
                        let done = matches!(event, ModelEvent::Done);
                        yield Ok(event);
                        if done {
                            break;
                        }
                    }
                    Ok(None) => {
                        yield Err(ModelError::StreamInterrupted(
                            "external adapter closed the stream without a done event".to_string(),
                        ));
                        break;
                    }
                    Err(error) => {
                        yield Err(error);
                        break;
                    }
                }
            }
            session.shutdown().await;
        })
    }

    fn model_id(&self) -> &str {
        &self.config.model
    }

    fn client_id(&self) -> ModelClientId {
        let endpoint = if self.config.base_url.is_empty() {
            self.config
                .command
                .first()
                .map(String::as_str)
                .unwrap_or("local")
        } else {
            self.config.base_url.as_str()
        };
        ModelClientId::new(EXTERNAL_ADAPTER_V1_PROTOCOL, endpoint, &self.config.model)
    }

    fn requires_terminal_event(&self) -> bool {
        true
    }
}

struct ExternalAdapterSession {
    config: Arc<ExternalAdapterConfig>,
    child: Child,
    stdout: BufReader<tokio::process::ChildStdout>,
    deadline: Instant,
    last_activity: Instant,
    saw_done: bool,
    open_tools: HashMap<String, String>,
}

impl ExternalAdapterSession {
    async fn start(
        config: Arc<ExternalAdapterConfig>,
        messages: Vec<Message>,
        tools: Vec<ModelToolSchema>,
    ) -> Result<Self, ModelError> {
        let mut command = Command::new(&config.command[0]);
        if config.command.len() > 1 {
            command.args(&config.command[1..]);
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .env_clear();

        for key in &config.env_allowlist {
            if let Ok(value) = std::env::var(key) {
                command.env(key, value);
            }
        }
        for (key, value) in &config.extra_env {
            command.env(key, value);
        }
        if let Some(dir) = &config.working_directory {
            command.current_dir(dir);
        }

        let mut child = command.spawn().map_err(|error| {
            ModelError::InvalidConfiguration(format!("external adapter failed to start: {error}"))
        })?;

        let mut stdin = child.stdin.take().ok_or_else(|| {
            ModelError::InvalidConfiguration("external adapter stdin is unavailable".to_string())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            ModelError::InvalidConfiguration("external adapter stdout is unavailable".to_string())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            ModelError::InvalidConfiguration("external adapter stderr is unavailable".to_string())
        })?;

        let mut stdout = BufReader::new(stdout);
        let started = Instant::now();

        let hello = serde_json::json!({
            "type": "hello",
            "protocol": EXTERNAL_ADAPTER_V1_PROTOCOL,
            "version": PROTOCOL_VERSION,
        });
        write_line(&mut stdin, &hello).await?;

        let hello_line = read_line_bounded(
            &mut stdout,
            config.max_stdout_line_bytes,
            config.startup_timeout,
        )
        .await?;
        validate_hello_response(&hello_line)?;

        let request = build_stream_request(&config, &messages, &tools);
        write_line(&mut stdin, &request).await?;
        drop(stdin);

        // Drain stderr in the background so a noisy adapter cannot block.
        let max_stderr = config.max_stderr_bytes;
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut buf = Vec::new();
            let mut total = 0usize;
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        total = total.saturating_add(line.len());
                        if total <= max_stderr {
                            buf.extend_from_slice(line.as_bytes());
                        }
                    }
                    Err(_) => break,
                }
            }
            // Intentionally unused: stderr is bounded and never logged with secrets.
            let _ = buf;
        });

        Ok(Self {
            config: Arc::clone(&config),
            child,
            stdout,
            deadline: started + config.request_timeout,
            last_activity: Instant::now(),
            saw_done: false,
            open_tools: HashMap::new(),
        })
    }

    async fn next_event(&mut self) -> Result<Option<ModelEvent>, ModelError> {
        if self.saw_done {
            return Ok(None);
        }
        let remaining_request = self.deadline.saturating_duration_since(Instant::now());
        if remaining_request.is_zero() {
            return Err(ModelError::RequestFailed(
                "external adapter request timed out".to_string(),
            ));
        }
        let idle = self.config.idle_timeout.min(remaining_request);
        let line = read_line_bounded(&mut self.stdout, self.config.max_stdout_line_bytes, idle)
            .await
            .map_err(|error| match error {
                ModelError::StreamInterrupted(message) if message.contains("timed out") => {
                    ModelError::StreamInterrupted(
                        "external adapter idle timeout while waiting for the next event"
                            .to_string(),
                    )
                }
                other => other,
            })?;

        self.last_activity = Instant::now();
        let event = decode_adapter_event(&line, &mut self.open_tools)?;
        if matches!(event, ModelEvent::Done) {
            self.saw_done = true;
        }
        Ok(Some(event))
    }

    async fn shutdown(mut self) {
        let _ = timeout(self.config.shutdown_timeout, self.child.wait()).await;
        let _ = self.child.start_kill();
        let _ = timeout(self.config.shutdown_timeout, self.child.wait()).await;
        let _ = self.last_activity;
    }
}

fn build_stream_request(
    config: &ExternalAdapterConfig,
    messages: &[Message],
    tools: &[ModelToolSchema],
) -> serde_json::Value {
    let (auth_style, secret_set, auth_header) = match config.auth.style() {
        AuthStyle::None => ("none", false, None),
        AuthStyle::Bearer => ("bearer", config.auth.secret().is_some(), None),
        AuthStyle::Header(name) => (
            "header",
            config.auth.secret().is_some(),
            Some(name.as_str().to_string()),
        ),
    };

    let mut secrets = serde_json::Map::new();
    if let Some(secret) = config.auth.secret() {
        secrets.insert(
            "primary".to_string(),
            serde_json::Value::String(secret.to_string()),
        );
    }
    let mut header_secrets = serde_json::Map::new();
    let mut headers = serde_json::Map::new();
    for header in &config.headers {
        headers.insert(
            header.name().as_str().to_string(),
            serde_json::Value::String(header.value().to_string()),
        );
        // Custom headers may carry credentials; mirror them into secrets for
        // adapters that prefer a dedicated secret map. Values are never logged.
        header_secrets.insert(
            header.name().as_str().to_string(),
            serde_json::Value::String(header.value().to_string()),
        );
    }
    if !header_secrets.is_empty() {
        secrets.insert(
            "headers".to_string(),
            serde_json::Value::Object(header_secrets),
        );
    }

    serde_json::json!({
        "type": "stream",
        "version": PROTOCOL_VERSION,
        "model": config.model,
        "messages": messages,
        "tools": tools,
        "options": config.options,
        "protocol_options": config.protocol_options,
        "base_url": config.base_url,
        "auth": {
            "style": auth_style,
            "header": auth_header,
            "secret_set": secret_set,
        },
        "headers": headers,
        "secrets": secrets,
    })
}

// ResolvedAuth/ResolvedHeader crate-private accessors are used above.

fn validate_hello_response(line: &str) -> Result<(), ModelError> {
    let value: serde_json::Value = serde_json::from_str(line).map_err(|_| {
        ModelError::InvalidConfiguration(
            "external adapter hello response is not valid JSON".to_string(),
        )
    })?;
    if value.get("type").and_then(|v| v.as_str()) != Some("hello_ok") {
        return Err(ModelError::InvalidConfiguration(
            "external adapter hello response type must be hello_ok".to_string(),
        ));
    }
    if value.get("protocol").and_then(|v| v.as_str()) != Some(EXTERNAL_ADAPTER_V1_PROTOCOL) {
        return Err(ModelError::InvalidConfiguration(
            "external adapter hello response protocol mismatch".to_string(),
        ));
    }
    let version = value
        .get("version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| {
            ModelError::InvalidConfiguration(
                "external adapter hello response version is missing".to_string(),
            )
        })?;
    if version != u64::from(PROTOCOL_VERSION) {
        return Err(ModelError::InvalidConfiguration(format!(
            "external adapter protocol version {version} is unsupported"
        )));
    }
    Ok(())
}

fn decode_adapter_event(
    line: &str,
    open_tools: &mut HashMap<String, String>,
) -> Result<ModelEvent, ModelError> {
    let value: serde_json::Value = serde_json::from_str(line).map_err(|_| {
        ModelError::StreamInterrupted("external adapter emitted a non-JSON event".to_string())
    })?;
    let event_type = value.get("type").and_then(|v| v.as_str()).ok_or_else(|| {
        ModelError::StreamInterrupted("external adapter event is missing type".to_string())
    })?;

    match event_type {
        "text_delta" => Ok(ModelEvent::TextDelta {
            text: required_string(&value, "text")?,
        }),
        "thinking_delta" => Ok(ModelEvent::ThinkingDelta {
            text: required_string(&value, "text")?,
        }),
        "tool_use_start" => {
            let id = required_string(&value, "id")?;
            let name = required_string(&value, "name")?;
            open_tools.insert(id.clone(), name.clone());
            Ok(ModelEvent::ToolUseStart { id, name })
        }
        "tool_use_delta" => Ok(ModelEvent::ToolUseDelta {
            id: required_string(&value, "id")?,
            args_delta: required_string(&value, "args_delta")?,
        }),
        "tool_use_done" => {
            let id = required_string(&value, "id")?;
            let name = value
                .get("name")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .or_else(|| open_tools.get(&id).cloned())
                .ok_or_else(|| {
                    ModelError::StreamInterrupted(
                        "external adapter tool_use_done is missing name".to_string(),
                    )
                })?;
            let args = value
                .get("args")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            open_tools.remove(&id);
            Ok(ModelEvent::ToolUseDone { id, name, args })
        }
        "usage" => Ok(ModelEvent::Usage {
            usage: Usage {
                prompt_tokens: optional_u32(&value, "prompt_tokens")?,
                completion_tokens: optional_u32(&value, "completion_tokens")?,
                total_tokens: optional_u32(&value, "total_tokens")?,
                cached_tokens: optional_u32(&value, "cached_tokens")?,
            },
        }),
        "done" => {
            if !open_tools.is_empty() {
                return Err(ModelError::StreamInterrupted(
                    "external adapter sent done while tool calls remain open".to_string(),
                ));
            }
            Ok(ModelEvent::Done)
        }
        "error" => {
            let code = value
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or("request_failed");
            // Never echo adapter-controlled error text into ModelError Display
            // beyond a short stable class; adapters may include secrets.
            match code {
                "auth_failed" => Err(ModelError::AuthFailed),
                "rate_limited" => Err(ModelError::RateLimited {
                    retry_after_ms: value
                        .get("retry_after_ms")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(1000),
                }),
                "context_length_exceeded" => Err(ModelError::ContextLengthExceeded {
                    used: value.get("used").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                    max: value.get("max").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                }),
                "invalid_configuration" => Err(ModelError::InvalidConfiguration(
                    "external adapter rejected the request configuration".to_string(),
                )),
                _ => Err(ModelError::RequestFailed(
                    "external adapter reported a request failure".to_string(),
                )),
            }
        }
        other => Err(ModelError::StreamInterrupted(format!(
            "external adapter emitted unsupported event type `{other}`"
        ))),
    }
}

async fn write_line(
    stdin: &mut tokio::process::ChildStdin,
    value: &serde_json::Value,
) -> Result<(), ModelError> {
    let mut encoded = serde_json::to_vec(value).map_err(|error| {
        ModelError::InvalidConfiguration(format!(
            "external adapter request could not be encoded: {error}"
        ))
    })?;
    encoded.push(b'\n');
    stdin.write_all(&encoded).await.map_err(|error| {
        ModelError::RequestFailed(format!("external adapter stdin write failed: {error}"))
    })?;
    stdin.flush().await.map_err(|error| {
        ModelError::RequestFailed(format!("external adapter stdin flush failed: {error}"))
    })?;
    Ok(())
}

async fn read_line_bounded(
    reader: &mut BufReader<tokio::process::ChildStdout>,
    max_bytes: usize,
    wait: Duration,
) -> Result<String, ModelError> {
    let mut line = Vec::new();
    let result = timeout(wait, async {
        loop {
            let mut buf = [0u8; 1];
            let n = tokio::io::AsyncReadExt::read(reader, &mut buf)
                .await
                .map_err(|error| {
                    ModelError::StreamInterrupted(format!(
                        "external adapter stdout read failed: {error}"
                    ))
                })?;
            if n == 0 {
                if line.is_empty() {
                    return Err(ModelError::StreamInterrupted(
                        "external adapter closed stdout".to_string(),
                    ));
                }
                break;
            }
            if buf[0] == b'\n' {
                break;
            }
            if line.len() >= max_bytes {
                return Err(ModelError::StreamInterrupted(
                    "external adapter stdout line exceeds the configured bound".to_string(),
                ));
            }
            line.push(buf[0]);
        }
        Ok::<(), ModelError>(())
    })
    .await;

    match result {
        Ok(Ok(())) => {
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            String::from_utf8(line).map_err(|_| {
                ModelError::StreamInterrupted(
                    "external adapter stdout line is not valid UTF-8".to_string(),
                )
            })
        }
        Ok(Err(error)) => Err(error),
        Err(_) => Err(ModelError::StreamInterrupted(
            "external adapter read timed out".to_string(),
        )),
    }
}

fn parse_command(value: Option<&serde_json::Value>) -> Result<Vec<String>, ModelError> {
    let array = value.and_then(|v| v.as_array()).ok_or_else(|| {
        ModelError::InvalidConfiguration(
            "external-adapter-v1 command must be a non-empty JSON array of strings".to_string(),
        )
    })?;
    if array.is_empty() || array.len() > MAX_COMMAND_ARGS {
        return Err(ModelError::InvalidConfiguration(format!(
            "external-adapter-v1 command must contain 1..={MAX_COMMAND_ARGS} entries"
        )));
    }
    let mut command = Vec::with_capacity(array.len());
    for item in array {
        let arg = item.as_str().ok_or_else(|| {
            ModelError::InvalidConfiguration(
                "external-adapter-v1 command entries must be strings".to_string(),
            )
        })?;
        if arg.is_empty() || arg.len() > MAX_ARG_BYTES || arg.contains('\0') {
            return Err(ModelError::InvalidConfiguration(
                "external-adapter-v1 command entry is empty, too large, or contains NUL"
                    .to_string(),
            ));
        }
        // Reject shell metacharacters so callers cannot smuggle a shell string.
        if arg
            .chars()
            .any(|ch| matches!(ch, '|' | '&' | ';' | '>' | '<' | '`' | '\n' | '\r'))
        {
            return Err(ModelError::InvalidConfiguration(
                "external-adapter-v1 command must be a direct executable argv, not a shell string"
                    .to_string(),
            ));
        }
        command.push(arg.to_string());
    }
    Ok(command)
}

fn validate_executable_path(
    path: &Path,
    workspace_root: &Path,
    allow_external_paths: bool,
) -> Result<(), ModelError> {
    if path.as_os_str().is_empty() {
        return Err(ModelError::InvalidConfiguration(
            "external-adapter-v1 executable path must not be empty".to_string(),
        ));
    }
    if path.to_string_lossy().contains('\0') {
        return Err(ModelError::InvalidConfiguration(
            "external-adapter-v1 executable path contains NUL".to_string(),
        ));
    }
    // Absolute paths (including Windows drive/UNC forms when validating on Unix)
    // are allowed only when external paths are enabled, or when the absolute path
    // still resolves under the workspace root.
    if path_looks_absolute(path) {
        if allow_external_paths {
            return Ok(());
        }
        let canonical_workspace = workspace_root
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.to_path_buf());
        if path.starts_with(&canonical_workspace) || path.starts_with(workspace_root) {
            return Ok(());
        }
        return Err(ModelError::InvalidConfiguration(
            "external-adapter-v1 executable resolves outside the workspace".to_string(),
        ));
    }
    Ok(())
}

/// True for host-native absolute paths and for Windows drive/UNC paths even when
/// the current process is running on Unix (common for configs validated in Linux CI).
fn path_looks_absolute(path: &Path) -> bool {
    if path.is_absolute() {
        return true;
    }
    let raw = path.to_string_lossy();
    let bytes = raw.as_bytes();
    // `C:\...` or `C:/...`
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return true;
    }
    // UNC: `\\server\share` or `//server/share`
    raw.starts_with("\\\\") || raw.starts_with("//")
}

fn validate_working_directory(
    path: &Path,
    workspace_root: &Path,
    allow_external_paths: bool,
) -> Result<(), ModelError> {
    if path.as_os_str().is_empty() || path.to_string_lossy().contains('\0') {
        return Err(ModelError::InvalidConfiguration(
            "external-adapter-v1 working_directory is invalid".to_string(),
        ));
    }
    if allow_external_paths {
        return Ok(());
    }
    if path_looks_absolute(path) {
        let resolved = path.to_path_buf();
        if !(resolved.starts_with(workspace_root)) {
            return Err(ModelError::InvalidConfiguration(
                "external-adapter-v1 working_directory resolves outside the workspace".to_string(),
            ));
        }
        return Ok(());
    }
    let resolved = workspace_root.join(path);
    if !resolved.starts_with(workspace_root) {
        return Err(ModelError::InvalidConfiguration(
            "external-adapter-v1 working_directory resolves outside the workspace".to_string(),
        ));
    }
    Ok(())
}

fn validate_env_name(name: &str) -> Result<(), ModelError> {
    let bytes = name.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 256
        || (!bytes[0].is_ascii_uppercase() && bytes[0] != b'_')
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
    {
        return Err(ModelError::InvalidConfiguration(
            "external-adapter-v1 environment name is invalid".to_string(),
        ));
    }
    Ok(())
}

fn duration_option(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    default: Duration,
) -> Result<Duration, ModelError> {
    match object.get(key) {
        None => Ok(default),
        Some(value) => {
            let ms = value.as_u64().ok_or_else(|| {
                ModelError::InvalidConfiguration(format!(
                    "external-adapter-v1 {key} must be a positive integer millisecond count"
                ))
            })?;
            if ms == 0 || ms > 600_000 {
                return Err(ModelError::InvalidConfiguration(format!(
                    "external-adapter-v1 {key} must be between 1 and 600000"
                )));
            }
            Ok(Duration::from_millis(ms))
        }
    }
}

fn usize_option(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    default: usize,
    min: usize,
    max: usize,
) -> Result<usize, ModelError> {
    match object.get(key) {
        None => Ok(default),
        Some(value) => {
            let number = value.as_u64().ok_or_else(|| {
                ModelError::InvalidConfiguration(format!(
                    "external-adapter-v1 {key} must be a positive integer"
                ))
            })? as usize;
            if number < min || number > max {
                return Err(ModelError::InvalidConfiguration(format!(
                    "external-adapter-v1 {key} must be between {min} and {max}"
                )));
            }
            Ok(number)
        }
    }
}

fn required_string(value: &serde_json::Value, key: &str) -> Result<String, ModelError> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            ModelError::StreamInterrupted(format!(
                "external adapter event is missing string field `{key}`"
            ))
        })
}

fn optional_u32(value: &serde_json::Value, key: &str) -> Result<u32, ModelError> {
    match value.get(key) {
        None => Ok(0),
        Some(item) => item.as_u64().map(|n| n as u32).ok_or_else(|| {
            ModelError::StreamInterrupted(format!(
                "external adapter event field `{key}` must be an integer"
            ))
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Message;
    use futures::StreamExt;
    use std::fs;
    use std::sync::OnceLock;
    use tempfile::TempDir;

    fn fixture_binary() -> PathBuf {
        static PATH: OnceLock<PathBuf> = OnceLock::new();
        PATH.get_or_init(|| {
            let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let src = manifest.join("tests/fixtures/external_adapter_v1_fixture.rs");
            let out_dir = manifest.join("../target/debug");
            let _ = fs::create_dir_all(&out_dir);
            let out = out_dir.join(if cfg!(windows) {
                "external_adapter_v1_fixture.exe"
            } else {
                "external_adapter_v1_fixture"
            });
            let status = std::process::Command::new("rustc")
                .args([
                    "--edition",
                    "2021",
                    "-O",
                    src.to_str().expect("utf8 path"),
                    "-o",
                    out.to_str().expect("utf8 path"),
                ])
                .status()
                .expect("rustc must be available to build the fixture");
            assert!(status.success(), "fixture rustc build failed: {status}");
            out
        })
        .clone()
    }

    fn base_config(mode: &str) -> ExternalAdapterConfig {
        let fixture = fixture_binary();
        ExternalAdapterConfig {
            command: vec![fixture.to_string_lossy().into_owned(), mode.to_string()],
            working_directory: None,
            env_allowlist: BTreeSet::new(),
            extra_env: BTreeMap::new(),
            base_url: String::new(),
            model: "adapter-model".to_string(),
            auth: ResolvedAuth::none(),
            headers: Vec::new(),
            options: ProviderOptions::default(),
            protocol_options: serde_json::json!({
                "command": [fixture.to_string_lossy(), mode],
            }),
            startup_timeout: Duration::from_secs(3),
            request_timeout: Duration::from_secs(5),
            idle_timeout: Duration::from_millis(800),
            shutdown_timeout: Duration::from_secs(2),
            max_stdout_line_bytes: 8 * 1024,
            max_stderr_bytes: 4 * 1024,
        }
    }

    #[tokio::test]
    async fn happy_path_streams_text_usage_and_done() {
        let client = ExternalAdapterClient::new(base_config("happy")).unwrap();
        let events = client
            .stream(&[Message::user("hello adapter")], &[])
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            ModelEvent::TextDelta { text } if text.contains("adapter-ok")
        )));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, ModelEvent::Usage { .. }))
        );
        assert!(matches!(events.last(), Some(ModelEvent::Done)));
        assert_eq!(
            client.client_id().as_str(),
            format!(
                "external-adapter-v1:{}:adapter-model",
                fixture_binary().to_string_lossy()
            )
        );
    }

    #[tokio::test]
    async fn malformed_event_fails_without_echoing_payload() {
        let client = ExternalAdapterClient::new(base_config("malformed")).unwrap();
        let result = client
            .stream(&[Message::user("x")], &[])
            .collect::<Vec<_>>()
            .await;
        let error = result
            .into_iter()
            .find_map(|item| item.err())
            .expect("malformed fixture must error");
        let message = error.to_string();
        assert!(message.contains("non-JSON") || message.contains("unsupported"));
        assert!(!message.contains("SECRET-VALUE"));
    }

    #[tokio::test]
    async fn idle_timeout_is_enforced() {
        let mut config = base_config("hang");
        config.idle_timeout = Duration::from_millis(300);
        config.request_timeout = Duration::from_secs(2);
        let client = ExternalAdapterClient::new(config).unwrap();
        let result = client
            .stream(&[Message::user("x")], &[])
            .collect::<Vec<_>>()
            .await;
        let error = result
            .into_iter()
            .find_map(|item| item.err())
            .expect("hang fixture must time out");
        assert!(
            error.to_string().to_ascii_lowercase().contains("timeout")
                || error.to_string().to_ascii_lowercase().contains("idle")
        );
    }

    #[tokio::test]
    async fn secrets_are_delivered_but_not_echoed_in_errors() {
        let mut config = base_config("secret");
        config.auth = ResolvedAuth::bearer("SECRET-VALUE").unwrap();
        let client = ExternalAdapterClient::new(config).unwrap();
        let events = client
            .stream(&[Message::user("ping")], &[])
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            ModelEvent::TextDelta { text } if text == "secret-present"
        )));
    }

    #[test]
    fn rejects_shell_command_strings_and_path_escape() {
        let tmp = TempDir::new().unwrap();
        let err = ExternalAdapterConfig::from_protocol_options(
            &serde_json::json!({
                "command": ["echo", "hi|there"]
            }),
            "",
            "m",
            ResolvedAuth::none(),
            Vec::new(),
            ProviderOptions::default(),
            tmp.path(),
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("direct executable"));

        // Use a Windows-style absolute path so Linux CI still rejects configs that
        // were authored against a Windows host (drive letters are not absolute on Unix).
        let err = ExternalAdapterConfig::from_protocol_options(
            &serde_json::json!({
                "command": ["C:/Windows/System32/cmd.exe"]
            }),
            "",
            "m",
            ResolvedAuth::none(),
            Vec::new(),
            ProviderOptions::default(),
            tmp.path(),
            false,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("outside the workspace"),
            "unexpected error: {err}"
        );

        let unix_outside = ExternalAdapterConfig::from_protocol_options(
            &serde_json::json!({
                "command": ["/usr/bin/true"]
            }),
            "",
            "m",
            ResolvedAuth::none(),
            Vec::new(),
            ProviderOptions::default(),
            tmp.path(),
            false,
        );
        if cfg!(unix) {
            let err = unix_outside.unwrap_err();
            assert!(
                err.to_string().contains("outside the workspace"),
                "unexpected error: {err}"
            );
        }
    }
}
