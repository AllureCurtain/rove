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
        let output = self.registry.execute(name, args.clone()).await?;

        // Step 6: result wrapping
        let result = ToolResult {
            call_id,
            output: output.content,
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
    if schema.get("type").and_then(|value| value.as_str()) == Some("object") && !args.is_object() {
        return Err(ToolError::InvalidArgs {
            reason: "tool arguments must be a JSON object".to_string(),
        });
    }

    let Some(required) = schema.get("required").and_then(|value| value.as_array()) else {
        return Ok(());
    };

    for field in required {
        let Some(field_name) = field.as_str() else {
            continue;
        };
        if args.get(field_name).is_none() {
            return Err(ToolError::InvalidArgs {
                reason: format!("Missing required argument: {field_name}"),
            });
        }
    }

    let Some(properties) = schema.get("properties").and_then(|value| value.as_object()) else {
        return Ok(());
    };

    for (field_name, field_schema) in properties {
        let Some(value) = args.get(field_name) else {
            continue;
        };
        let Some(expected_type) = field_schema.get("type").and_then(|value| value.as_str()) else {
            continue;
        };

        if !value_matches_schema_type(value, expected_type) {
            return Err(ToolError::InvalidArgs {
                reason: format!("Argument {field_name} must be {expected_type}"),
            });
        }
    }

    Ok(())
}

fn value_matches_schema_type(value: &serde_json::Value, expected_type: &str) -> bool {
    match expected_type {
        "array" => value.is_array(),
        "boolean" => value.is_boolean(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "object" => value.is_object(),
        "string" => value.is_string(),
        _ => true,
    }
}
