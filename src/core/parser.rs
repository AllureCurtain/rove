use crate::core::types::{Action, CallId, ToolCallAction};

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
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw.trim()) {
        if let Some(calls) = parse_tool_batch(&value) {
            return calls_to_action(calls);
        }

        if let (Some(name), Some(args)) = (
            value.get("tool").and_then(|v| v.as_str()),
            value.get("args"),
        ) {
            return Action::ToolCall {
                call_id: CallId::new(),
                tool_use_id: None,
                name: name.to_string(),
                args: args.clone(),
            };
        }
    }

    // Default: treat as final answer
    Action::Final {
        text: raw.to_string(),
    }
}

fn parse_tool_batch(value: &serde_json::Value) -> Option<Vec<ToolCallAction>> {
    let calls = value
        .get("tools")
        .or_else(|| value.get("tool_calls"))
        .and_then(|calls| calls.as_array())?;
    if calls.is_empty() {
        return None;
    }

    let mut parsed = Vec::with_capacity(calls.len());
    for call in calls {
        let name = call
            .get("tool")
            .or_else(|| call.get("name"))
            .and_then(|value| value.as_str())?;
        let args = call.get("args").cloned().unwrap_or(serde_json::Value::Null);
        parsed.push(ToolCallAction {
            call_id: CallId::new(),
            tool_use_id: None,
            name: name.to_string(),
            args,
        });
    }
    Some(parsed)
}

fn calls_to_action(mut calls: Vec<ToolCallAction>) -> Action {
    if calls.len() == 1 {
        let call = calls.remove(0);
        Action::ToolCall {
            call_id: call.call_id,
            tool_use_id: call.tool_use_id,
            name: call.name,
            args: call.args,
        }
    } else {
        Action::ToolBatch { calls }
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

    #[test]
    fn parse_tool_batch_json() {
        let input = r#"{"tools":[{"tool":"echo","args":{"message":"a"}},{"tool":"echo","args":{"message":"b"}}]}"#;
        let action = parse_action(input);
        match action {
            Action::ToolBatch { calls } => {
                assert_eq!(calls.len(), 2);
                assert_eq!(calls[0].name, "echo");
                assert_eq!(calls[0].args["message"], "a");
                assert_eq!(calls[1].name, "echo");
                assert_eq!(calls[1].args["message"], "b");
            }
            _ => panic!("Expected ToolBatch action"),
        }
    }
}
