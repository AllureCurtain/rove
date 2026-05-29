use crate::core::boundary::check_tool_allowed;
use crate::core::types::{CallId, ToolContext, ToolResult};
use crate::errors::ToolError;
use crate::hooks::{HookRegistry, PostToolHookContext};
use crate::tools::registry::ToolRegistry;

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
        // Step 1: schema lookup
        let schema = self.registry.schema(name)?;

        // Step 2: input validation
        validate_args(&schema.parameters, &args)?;

        // Step 3: pre-tool hooks
        self.hooks.run_pre_tool(ctx, name, &args).await?;

        // Step 4: permission boundary
        check_tool_allowed(&schema, ctx.approval_policy)?;

        // Step 5: execute
        let output = self.registry.execute(name, args.clone(), ctx).await?;

        // Step 6: result wrapping
        let result = ToolResult {
            call_id,
            output: output.content,
            mutations: output.mutations,
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

    let reason = if path.is_root() && expected_type == "object" {
        "tool arguments must be a JSON object".to_string()
    } else {
        format!("Argument {} must be {expected_type}", path.display())
    };
    invalid_args(reason)
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
