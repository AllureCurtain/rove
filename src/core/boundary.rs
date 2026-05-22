use crate::core::types::{ApprovalPolicy, ToolSchema};
use crate::errors::ToolError;

/// Check whether a tool call is allowed under the active approval policy.
pub fn check_tool_allowed(schema: &ToolSchema, policy: ApprovalPolicy) -> Result<(), ToolError> {
    match (schema.destructive, policy) {
        (true, ApprovalPolicy::Never) => Err(ToolError::PermissionDenied {
            reason: "destructive tool blocked by policy".to_string(),
        }),
        (true, ApprovalPolicy::Ask) => Err(ToolError::PermissionDenied {
            reason: "destructive tool requires explicit approval".to_string(),
        }),
        _ => Ok(()),
    }
}
