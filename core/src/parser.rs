use crate::{Action, CallId, ToolCallAction};

/// Parse the compatibility JSON tool-call form, otherwise return final text.
pub fn parse_action(raw: &str) -> Action {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw.trim()) {
        if let Some(calls) = parse_tool_batch(&value) {
            return calls_to_action(calls);
        }

        if let (Some(name), Some(args)) = (
            value.get("tool").and_then(serde_json::Value::as_str),
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

    Action::Final {
        text: raw.to_string(),
    }
}

fn parse_tool_batch(value: &serde_json::Value) -> Option<Vec<ToolCallAction>> {
    let calls = value
        .get("tools")
        .or_else(|| value.get("tool_calls"))
        .and_then(serde_json::Value::as_array)?;
    if calls.is_empty() {
        return None;
    }

    let mut parsed = Vec::with_capacity(calls.len());
    for call in calls {
        let name = call
            .get("tool")
            .or_else(|| call.get("name"))
            .and_then(serde_json::Value::as_str)?;
        parsed.push(ToolCallAction {
            call_id: CallId::new(),
            tool_use_id: None,
            name: name.to_string(),
            args: call.get("args").cloned().unwrap_or(serde_json::Value::Null),
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
    use super::parse_action;
    use crate::Action;

    #[test]
    fn parses_text_and_compatibility_tool_calls() {
        assert!(matches!(parse_action("done"), Action::Final { text } if text == "done"));
        assert!(matches!(
            parse_action(r#"{"tool":"echo","args":{"message":"hi"}}"#),
            Action::ToolCall { name, .. } if name == "echo"
        ));
        assert!(matches!(
            parse_action(r#"{"tools":[{"tool":"a","args":{}},{"tool":"b","args":{}}]}"#),
            Action::ToolBatch { calls } if calls.len() == 2
        ));
    }
}
