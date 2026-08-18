use crate::boundary::check_tool_allowed;
use crate::hooks::{HookRegistry, PostToolHookContext};
use crate::review::{descriptor_allowed, is_review_mode};
use crate::tool_input::RegisteredUserInput;
use crate::tools::runtime_context::runtime_tool_services;
use crate::types::{
    CallId, ToolContext, ToolDescriptor, ToolExecutionMetadata, ToolMutation, ToolResult,
    ToolRiskLevel,
};
use rove_core::{
    ArtifactTrust, Sensitivity, ToolArtifactKind, ToolArtifactSource, ToolError,
    ToolOutputEnvelope, ToolRegistry, ToolResultOutcome,
};
use tokio::sync::mpsc;

/// The executor runs tools through the pipeline.
///
/// M0 pipeline (simplified):
///   1. Look up tool in registry
///   2. Execute
///
/// M1 pipeline (full):
///   schema → validate_input → pre-hook → permission → exec → post-hook → diff
pub struct Executor<'a> {
    registry: &'a ToolRegistry,
    hooks: HookRegistry,
}

impl<'a> Executor<'a> {
    pub fn new(registry: &'a ToolRegistry) -> Self {
        Self::with_hooks(registry, HookRegistry::default())
    }

    pub fn with_hooks(registry: &'a ToolRegistry, hooks: HookRegistry) -> Self {
        Self { registry, hooks }
    }

    /// Execute a tool call through the pipeline.
    pub async fn run(
        &self,
        ctx: &ToolContext<'_>,
        name: &str,
        args: serde_json::Value,
        call_id: CallId,
    ) -> Result<ToolResult, ToolError> {
        self.run_with_input_events(ctx, name, args, call_id, None)
            .await
    }

    pub async fn run_with_input_events(
        &self,
        ctx: &ToolContext<'_>,
        name: &str,
        args: serde_json::Value,
        call_id: CallId,
        input_events: Option<mpsc::Sender<RegisteredUserInput>>,
    ) -> Result<ToolResult, ToolError> {
        // Step 1: schema lookup
        let schema = self.registry.descriptor(name)?;

        // Step 2: input validation
        validate_args(&schema.parameters, &args)?;

        // Review authorization is checked before hooks and before any
        // permission/approval path can observe or influence the call.
        let services = runtime_tool_services(ctx)?;
        if is_review_mode(services.run_mode) && !descriptor_allowed(&schema) {
            return Err(ToolError::PermissionDenied {
                reason: "review mode forbids non-read-only tool".to_string(),
            });
        }

        // Step 3: pre-tool hooks
        self.hooks.run_pre_tool(ctx, name, &args).await?;

        // Step 4: permission boundary
        check_tool_allowed(&schema, services.approval_policy)?;

        // Step 5: execute
        let mut output = crate::tool_input::scope(
            call_id,
            input_events,
            self.registry.execute(name, args.clone(), ctx),
        )
        .await?;
        retain_eligible_output(ctx, name, &mut output).await;
        let metadata = completion_metadata(&schema, &output.mutations, output.outcome());

        // Step 6: result wrapping
        let result = ToolResult {
            call_id,
            output: output.content,
            mutations: output.mutations,
            metadata,
            envelope: output.envelope,
        };

        // Step 7: post-tool hooks
        self.hooks
            .run_post_tool(&PostToolHookContext {
                tool_context: ctx,
                name,
                args: &args,
                result: &result,
            })
            .await;

        Ok(result)
    }
}

async fn retain_eligible_output(
    ctx: &ToolContext<'_>,
    name: &str,
    output: &mut rove_core::ToolOutput,
) {
    if !matches!(
        name,
        "read_file" | "search_code" | "list_directory" | "glob_paths"
    ) || output.content.is_empty()
    {
        return;
    }
    let Ok(services) = runtime_tool_services(ctx) else {
        return;
    };
    // Review source text is intentionally available to the in-process model,
    // but the immutable snapshot is its only durable source authority. Do not
    // duplicate read/search payloads into the run's Tool Artifact store.
    if is_review_mode(services.run_mode) {
        return;
    }
    let Some(store) = services.tool_artifacts.as_ref() else {
        return;
    };
    let sensitivity = local_tool_output_sensitivity(&output.content);
    if sensitivity == Sensitivity::Sensitive {
        return;
    }
    let source = ToolArtifactSource {
        run_id: store.run_id(),
        call_id: ctx.call_id.to_string(),
        remote_tool_name: Some(name.to_string()),
        captured_at: chrono::Utc::now().to_rfc3339(),
        ..ToolArtifactSource::default()
    };
    let Ok(artifact) = store
        .put(
            ToolArtifactKind::Text,
            output.content.as_bytes(),
            source,
            crate::state::tool_artifacts::ArtifactClaim {
                mime_type: Some("text/plain; charset=utf-8".to_string()),
                ..crate::state::tool_artifacts::ArtifactClaim::default()
            },
            sensitivity,
            ArtifactTrust::LocalTool,
        )
        .await
    else {
        return;
    };
    let mut envelope = output
        .envelope
        .take()
        .map(|envelope| *envelope)
        .unwrap_or_else(|| ToolOutputEnvelope::text(output.content.clone()));
    if envelope
        .artifacts
        .iter()
        .all(|existing| existing.artifact_id != artifact.artifact_id)
    {
        envelope.artifacts.push(artifact);
    }
    output.envelope = Some(Box::new(envelope));
}

fn local_tool_output_sensitivity(content: &str) -> Sensitivity {
    let lower = content.to_ascii_lowercase();
    let sensitive_path = lower.contains("\"path\":\".env")
        || lower.contains("\"path\":\".git/")
        || lower.contains("\"path\":\".rove/")
        || lower.contains(".pem\"")
        || lower.contains(".key\"");
    let secret_assignment = content.lines().any(|line| {
        let line = line.trim();
        let Some((name, value)) = line.split_once('=') else {
            return false;
        };
        let name = name.to_ascii_uppercase();
        !value.trim().is_empty()
            && ["SECRET", "TOKEN", "PASSWORD", "API_KEY", "PRIVATE_KEY"]
                .iter()
                .any(|marker| name.contains(marker))
    });
    if sensitive_path || secret_assignment {
        Sensitivity::Sensitive
    } else {
        Sensitivity::Normal
    }
}

fn validate_args(schema: &serde_json::Value, args: &serde_json::Value) -> Result<(), ToolError> {
    validate_schema_value(schema, args, JsonPath::root())
}

fn validate_schema_value(
    schema: &serde_json::Value,
    value: &serde_json::Value,
    path: JsonPath,
) -> Result<(), ToolError> {
    if let Some(expected_type) = schema.get("type").and_then(|value| value.as_str()) {
        validate_type(value, expected_type, &path)?;
    }

    if let Some(enum_values) = schema.get("enum").and_then(|value| value.as_array())
        && !enum_values.iter().any(|enum_value| enum_value == value)
    {
        return invalid_args(format!(
            "Argument {} must match one of the enum values",
            path.display()
        ));
    }

    validate_string_constraints(schema, value, &path)?;
    validate_numeric_constraints(schema, value, &path)?;

    match schema.get("type").and_then(|value| value.as_str()) {
        Some("object") => validate_object(schema, value, path),
        Some("array") => validate_array(schema, value, path),
        _ => Ok(()),
    }
}

fn validate_object(
    schema: &serde_json::Value,
    value: &serde_json::Value,
    path: JsonPath,
) -> Result<(), ToolError> {
    let Some(object) = value.as_object() else {
        return Ok(());
    };

    if let Some(required) = schema.get("required").and_then(|value| value.as_array()) {
        for field in required {
            let Some(field_name) = field.as_str() else {
                continue;
            };
            if !object.contains_key(field_name) {
                return invalid_args(format!(
                    "Missing required argument: {}",
                    path.child(field_name).display()
                ));
            }
        }
    }

    let properties = schema.get("properties").and_then(|value| value.as_object());
    if schema
        .get("additionalProperties")
        .and_then(|value| value.as_bool())
        == Some(false)
    {
        for field_name in object.keys() {
            if !properties
                .map(|properties| properties.contains_key(field_name))
                .unwrap_or(false)
            {
                return invalid_args(format!(
                    "Argument {} is not allowed by additionalProperties=false",
                    path.child(field_name).display()
                ));
            }
        }
    }

    if let Some(properties) = properties {
        for (field_name, field_schema) in properties {
            let Some(child_value) = object.get(field_name) else {
                continue;
            };
            validate_schema_value(field_schema, child_value, path.child(field_name))?;
        }
    }

    Ok(())
}

fn validate_array(
    schema: &serde_json::Value,
    value: &serde_json::Value,
    path: JsonPath,
) -> Result<(), ToolError> {
    let Some(items) = value.as_array() else {
        return Ok(());
    };

    if let Some(min_items) = schema.get("minItems").and_then(|value| value.as_u64())
        && items.len() < min_items as usize
    {
        return invalid_args(format!(
            "Argument {} must have at least {min_items} items",
            path.display()
        ));
    }

    if let Some(max_items) = schema.get("maxItems").and_then(|value| value.as_u64())
        && items.len() > max_items as usize
    {
        return invalid_args(format!(
            "Argument {} must have at most {max_items} items",
            path.display()
        ));
    }

    if let Some(item_schema) = schema.get("items") {
        for (index, item) in items.iter().enumerate() {
            validate_schema_value(item_schema, item, path.index(index))?;
        }
    }

    Ok(())
}

fn validate_type(
    value: &serde_json::Value,
    expected_type: &str,
    path: &JsonPath,
) -> Result<(), ToolError> {
    if value_matches_schema_type(value, expected_type) {
        return Ok(());
    }

    let received_type = json_type(value);
    let correction = deterministic_correction(path, expected_type, value);
    let reason = format!(
        "field '{}' must be a JSON {expected_type}; received {received_type} {}. Retry with {correction}.",
        path.display(),
        bounded_json(value, 96),
    );
    invalid_args(reason)
}

fn json_type(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

fn bounded_json(value: &serde_json::Value, max: usize) -> String {
    let encoded = serde_json::to_string(value).unwrap_or_else(|_| "<unserializable>".to_string());
    rove_core::truncate_utf8(&encoded, max).0
}

fn deterministic_correction(
    path: &JsonPath,
    expected_type: &str,
    value: &serde_json::Value,
) -> String {
    let corrected = match (expected_type, value) {
        ("boolean", serde_json::Value::String(text)) if text == "true" => serde_json::json!(true),
        ("boolean", serde_json::Value::String(text)) if text == "false" => serde_json::json!(false),
        ("integer", serde_json::Value::String(text)) => text
            .parse::<i64>()
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::from(0)),
        ("string", other) => serde_json::Value::String(other.to_string()),
        ("array", _) => serde_json::json!([]),
        ("object", _) => serde_json::json!({}),
        ("boolean", _) => serde_json::json!(false),
        ("integer", _) | ("number", _) => serde_json::json!(0),
        ("null", _) => serde_json::Value::Null,
        _ => serde_json::Value::Null,
    };
    if path.is_root() {
        bounded_json(&corrected, 160)
    } else {
        bounded_json(&serde_json::json!({path.display(): corrected}), 160)
    }
}

fn validate_string_constraints(
    schema: &serde_json::Value,
    value: &serde_json::Value,
    path: &JsonPath,
) -> Result<(), ToolError> {
    let Some(text) = value.as_str() else {
        return Ok(());
    };

    if let Some(min_length) = schema.get("minLength").and_then(|value| value.as_u64())
        && text.chars().count() < min_length as usize
    {
        return invalid_args(format!(
            "Argument {} must have at least {min_length} characters",
            path.display()
        ));
    }

    if let Some(max_length) = schema.get("maxLength").and_then(|value| value.as_u64())
        && text.chars().count() > max_length as usize
    {
        return invalid_args(format!(
            "Argument {} must have at most {max_length} characters",
            path.display()
        ));
    }

    Ok(())
}

fn validate_numeric_constraints(
    schema: &serde_json::Value,
    value: &serde_json::Value,
    path: &JsonPath,
) -> Result<(), ToolError> {
    let Some(number) = value.as_f64() else {
        return Ok(());
    };

    if let Some(minimum) = schema.get("minimum").and_then(|value| value.as_f64())
        && number < minimum
    {
        return invalid_args(format!(
            "Argument {} must be greater than or equal to minimum {minimum}",
            path.display()
        ));
    }

    if let Some(maximum) = schema.get("maximum").and_then(|value| value.as_f64())
        && number > maximum
    {
        return invalid_args(format!(
            "Argument {} must be less than or equal to maximum {maximum}",
            path.display()
        ));
    }

    Ok(())
}

fn value_matches_schema_type(value: &serde_json::Value, expected_type: &str) -> bool {
    match expected_type {
        "array" => value.is_array(),
        "boolean" => value.is_boolean(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "null" => value.is_null(),
        "number" => value.is_number(),
        "object" => value.is_object(),
        "string" => value.is_string(),
        _ => true,
    }
}

fn invalid_args<T>(reason: String) -> Result<T, ToolError> {
    Err(ToolError::InvalidArgs { reason })
}

/// Metadata for a tool that returned rather than failed.
///
/// The status comes from the rich outcome, not from the fact that the call
/// returned: a partial, cancelled, or indeterminate envelope must not be
/// recorded as a plain success.
fn completion_metadata(
    schema: &ToolDescriptor,
    mutations: &[ToolMutation],
    outcome: ToolResultOutcome,
) -> ToolExecutionMetadata {
    ToolExecutionMetadata {
        status: outcome.to_execution_status(),
        error_code: (outcome != ToolResultOutcome::Success).then(|| match outcome {
            ToolResultOutcome::Partial => "tool_partial_result".to_string(),
            ToolResultOutcome::Rejected => "tool_rejected".to_string(),
            ToolResultOutcome::Cancelled => "tool_cancelled".to_string(),
            ToolResultOutcome::TimedOutKnownNotSent => "tool_timed_out_not_sent".to_string(),
            ToolResultOutcome::Indeterminate => "tool_indeterminate_effect".to_string(),
            ToolResultOutcome::Error | ToolResultOutcome::Success => "tool_error".to_string(),
        }),
        security_event_type: None,
        risk_level: if schema.destructive {
            ToolRiskLevel::High
        } else {
            ToolRiskLevel::Low
        },
        read_only: !schema.destructive,
        affected_paths: mutations
            .iter()
            .map(|mutation| mutation.path.clone())
            .collect(),
        workspace_changed: !mutations.is_empty(),
        diff_summary: mutations
            .iter()
            .map(|mutation| format!("{:?}: {}", mutation.operation, mutation.path))
            .collect(),
    }
}

#[derive(Debug, Clone)]
struct JsonPath(String);

impl JsonPath {
    fn root() -> Self {
        Self(String::new())
    }

    fn child(&self, name: &str) -> Self {
        if self.0.is_empty() {
            Self(name.to_string())
        } else {
            Self(format!("{}.{}", self.0, name))
        }
    }

    fn index(&self, index: usize) -> Self {
        Self(format!("{}[{index}]", self.display()))
    }

    fn display(&self) -> &str {
        if self.0.is_empty() {
            "arguments"
        } else {
            &self.0
        }
    }

    fn is_root(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use tokio_util::sync::CancellationToken;

    use super::{Executor, local_tool_output_sensitivity};
    use crate::environment::local_environment;
    use crate::memory::paths::MemoryPaths;
    use crate::state::tool_artifacts::ToolArtifactStore;
    use crate::tools::runtime_context::{
        runtime_tool_context, runtime_tool_context_with_mode_and_artifacts,
    };
    use crate::types::{
        ApprovalPolicy, CallId, RunId, RunMode, ToolContext, ToolDescriptor, ToolExecutionStatus,
        ToolMutation, ToolMutationOperation, ToolRiskLevel,
    };
    use crate::workspace::Workspace;
    use rove_core::{Sensitivity, Tool, ToolOutput, ToolRegistry};

    struct MutatingTool;

    struct BooleanTool;

    struct ReadFileTool;

    #[async_trait]
    impl Tool for ReadFileTool {
        fn schema(&self) -> ToolDescriptor {
            ToolDescriptor {
                name: "read_file".to_string(),
                description: "Read a file".to_string(),
                parameters: serde_json::json!({"type": "object"}),
                destructive: false,
                parallel_safe: true,
                capability_id: Some("workspace.fs.read".to_string()),
                capability: None,
            }
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &ToolContext<'_>,
        ) -> Result<ToolOutput, rove_core::ToolError> {
            Ok(ToolOutput {
                content: "ordinary source text".to_string(),
                mutations: Vec::new(),
                envelope: None,
            })
        }
    }

    #[async_trait]
    impl Tool for BooleanTool {
        fn schema(&self) -> ToolDescriptor {
            ToolDescriptor {
                name: "search".to_string(),
                description: "Search".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": { "regex": { "type": "boolean" } },
                    "additionalProperties": false
                }),
                destructive: true,
                parallel_safe: false,
                capability_id: None,
                capability: None,
            }
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &ToolContext<'_>,
        ) -> Result<ToolOutput, rove_core::ToolError> {
            panic!("schema-invalid input must fail before permission or dispatch")
        }
    }

    #[async_trait]
    impl Tool for MutatingTool {
        fn schema(&self) -> ToolDescriptor {
            ToolDescriptor {
                name: "write_note".to_string(),
                description: "Write a note".to_string(),
                parameters: serde_json::json!({"type": "object"}),
                destructive: true,
                parallel_safe: false,
                capability_id: None,
                capability: None,
            }
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &ToolContext<'_>,
        ) -> Result<ToolOutput, rove_core::ToolError> {
            Ok(ToolOutput {
                content: "wrote note".to_string(),
                mutations: vec![ToolMutation {
                    path: "notes/today.md".to_string(),
                    operation: ToolMutationOperation::Update,
                    diff: Some("+hello".to_string()),
                }],
                envelope: None,
            })
        }
    }

    #[tokio::test]
    async fn successful_tool_result_includes_execution_metadata() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MutatingTool));
        let ctx = runtime_tool_context(
            CallId::new(),
            &workspace,
            MemoryPaths::from_workspace(&workspace, 8),
            ApprovalPolicy::Auto,
            None,
            CancellationToken::new(),
        );

        let result = Executor::new(&registry)
            .run(&ctx, "write_note", serde_json::json!({}), CallId::new())
            .await
            .unwrap();

        assert_eq!(result.metadata.status, ToolExecutionStatus::Ok);
        assert_eq!(result.metadata.risk_level, ToolRiskLevel::High);
        assert!(!result.metadata.read_only);
        assert!(result.metadata.workspace_changed);
        assert_eq!(result.metadata.affected_paths, vec!["notes/today.md"]);
        assert_eq!(result.metadata.diff_summary, vec!["Update: notes/today.md"]);
    }

    #[tokio::test]
    async fn type_error_names_field_types_value_and_bounded_correction_before_approval() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(BooleanTool));
        let ctx = runtime_tool_context(
            CallId::new(),
            &workspace,
            MemoryPaths::from_workspace(&workspace, 8),
            ApprovalPolicy::Never,
            None,
            CancellationToken::new(),
        );

        let error = Executor::new(&registry)
            .run(
                &ctx,
                "search",
                serde_json::json!({"regex":"true"}),
                CallId::new(),
            )
            .await
            .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("field 'regex'"));
        assert!(message.contains("JSON boolean"));
        assert!(message.contains("received string \"true\""));
        assert!(message.contains(r#"Retry with {"regex":true}"#));
        assert!(!message.contains("Permission denied"));
        assert!(message.len() < 512);
    }

    #[tokio::test]
    async fn review_read_output_is_not_retained_as_a_durable_tool_artifact() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(ReadFileTool));
        let store = Arc::new(ToolArtifactStore::new(
            tmp.path().join("runs").join(RunId::new().to_string()),
        ));
        let ctx = runtime_tool_context_with_mode_and_artifacts(
            CallId::new(),
            &workspace,
            MemoryPaths::from_workspace(&workspace, 8),
            ApprovalPolicy::Never,
            None,
            CancellationToken::new(),
            local_environment(&workspace),
            Some(Arc::clone(&store)),
            RunMode::Review,
        );

        let result = Executor::new(&registry)
            .run(&ctx, "read_file", serde_json::json!({}), CallId::new())
            .await
            .unwrap();

        assert_eq!(result.output, "ordinary source text");
        assert!(
            result
                .envelope
                .as_deref()
                .is_none_or(|envelope| envelope.artifacts.is_empty())
        );
        assert!(store.ledger().await.unwrap().is_empty());
    }

    #[test]
    fn secret_shaped_local_output_is_not_eligible_for_normal_retention() {
        assert_eq!(
            local_tool_output_sensitivity("API_KEY=live-value"),
            Sensitivity::Sensitive
        );
        assert_eq!(
            local_tool_output_sensitivity(r#"{"path":".env","content":"x"}"#),
            Sensitivity::Sensitive
        );
        assert_eq!(
            local_tool_output_sensitivity("API_KEY=live-value"),
            Sensitivity::Sensitive
        );
        assert_eq!(
            local_tool_output_sensitivity(r#"{"entries":[{"path":".rove/config.toml"}]}"#),
            Sensitivity::Sensitive
        );
        assert_eq!(
            local_tool_output_sensitivity("token is discussed here"),
            Sensitivity::Normal
        );
    }
}
