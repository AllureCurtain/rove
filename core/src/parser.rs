use crate::{Action, CallId, ToolCallAction};

/// Parse the compatibility JSON tool-call form, otherwise return final text.
///
/// A JSON object that advertises a tool shape but is malformed is represented
/// as a recoverable action. Plain JSON/text remains a normal final response so
/// compatibility providers can still answer naturally.
pub fn parse_action(raw: &str) -> Action {
    let trimmed = raw.trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(calls) = parse_tool_batch(&value) {
            return calls_to_action(calls);
        }

        if value.get("tools").is_some() || value.get("tool_calls").is_some() {
            return Action::Malformed {
                reason: "compatibility tool batch requires a non-empty array of calls with string names and object args".to_string(),
            };
        }

        if value.get("tool").is_some() || value.get("args").is_some() {
            let Some(name) = value.get("tool").and_then(serde_json::Value::as_str) else {
                return Action::Malformed {
                    reason: "compatibility tool output requires a string field 'tool'".to_string(),
                };
            };
            let Some(args) = value.get("args") else {
                return Action::Malformed {
                    reason: format!("compatibility call '{name}' is missing the 'args' field"),
                };
            };
            if !args.is_object() {
                return Action::Malformed {
                    reason: format!(
                        "compatibility call '{name}' requires 'args' to be a JSON object"
                    ),
                };
            }
            return Action::ToolCall {
                call_id: CallId::new(),
                tool_use_id: None,
                name: name.to_string(),
                args: args.clone(),
            };
        }
    }

    // A likely JSON/tool envelope which is not valid JSON must not be silently
    // promoted to a final answer. Keep the heuristic narrow to avoid changing
    // ordinary prose containing braces.
    if trimmed.starts_with('{')
        && (trimmed.contains("\"tool\"")
            || trimmed.contains("\"tools\"")
            || trimmed.contains("\"tool_calls\"")
            || trimmed.contains("\"args\""))
    {
        return Action::Malformed {
            reason:
                "compatibility tool output is not valid JSON; retry with a structured tool call"
                    .to_string(),
        };
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
        let args = call.get("args")?;
        if !args.is_object() {
            return None;
        }
        parsed.push(ToolCallAction {
            call_id: CallId::new(),
            tool_use_id: None,
            name: name.to_string(),
            args: args.clone(),
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

    #[test]
    fn malformed_compatibility_output_is_recoverable() {
        assert!(matches!(
            parse_action(r#"{"tool":"echo","args":"true"}"#),
            Action::Malformed { .. }
        ));
        assert!(matches!(
            parse_action(r#"{"tool":"echo"}"#),
            Action::Malformed { .. }
        ));
        assert!(matches!(
            parse_action(r#"{"tool":"echo","args":}"#),
            Action::Malformed { .. }
        ));
        assert!(matches!(
            parse_action(r#"{"tools":[{"tool":"echo","args":"bad"}]}"#),
            Action::Malformed { .. }
        ));
    }
}
