use crate::core::boundary::check_tool_allowed;
use crate::core::types::{CallId, ToolContext, ToolResult};
use crate::errors::ToolError;
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
}

impl<'a> Executor<'a> {
    pub fn new(registry: &'a ToolRegistry) -> Self {
        Self { registry }
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

        // Step 3: permission boundary
        check_tool_allowed(&schema, ctx.approval_policy)?;

        // Step 4: execute
        let output = self.registry.execute(name, args).await?;

        // Step 5: result wrapping
        Ok(ToolResult {
            call_id,
            output: output.content,
        })
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

    Ok(())
}
