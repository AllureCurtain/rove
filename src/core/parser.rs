use crate::core::types::Action;

/// Parse raw LLM output text into an Action.
///
/// For M0, we use a simple heuristic:
/// - If the output contains a tool_call JSON block, parse it as ToolCall.
/// - Otherwise, treat the entire output as a Final answer.
///
/// In M1+, this will be replaced by proper Anthropic/OpenAI tool_use block parsing
/// integrated with the streaming model client.
pub fn parse_action(raw: &str) -> Action {
    // Try to detect a tool call in the output.
    // Convention: LLM outputs JSON like {"tool": "name", "args": {...}}
    // This is a simplified parser for M0; real parsing happens at the model client level.
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw.trim())
        && let (Some(name), Some(args)) = (
            value.get("tool").and_then(|v| v.as_str()),
            value.get("args"),
        )
    {
        return Action::ToolCall {
            call_id: crate::core::types::CallId::new(),
            name: name.to_string(),
            args: args.clone(),
        };
    }

    // Default: treat as final answer
    Action::Final {
        text: raw.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plain_text_as_final() {
        let action = parse_action("Hello, world!");
        match action {
            Action::Final { text } => assert_eq!(text, "Hello, world!"),
            _ => panic!("Expected Final action"),
        }
    }

    #[test]
    fn parse_tool_call_json() {
        let input = r#"{"tool": "echo", "args": {"message": "hi"}}"#;
        let action = parse_action(input);
        match action {
            Action::ToolCall { name, args, .. } => {
                assert_eq!(name, "echo");
                assert_eq!(args["message"], "hi");
            }
            _ => panic!("Expected ToolCall action"),
        }
    }
}
