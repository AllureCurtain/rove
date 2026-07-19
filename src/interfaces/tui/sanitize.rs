//! Defensive, bounded projection helpers for TUI display surfaces.
//!
//! These helpers are deliberately conservative. They reduce common accidental
//! secret exposure (sensitive JSON keys and obvious token-bearing output), but
//! are not a guarantee that arbitrary user/tool text contains no secrets.

use serde_json::Value;

use super::state::MAX_TOOL_DETAIL_TEXT_BYTES;

pub(crate) fn truncate_display_text(value: &str, max_bytes: usize) -> String {
    let mut output = String::new();
    let mut truncated = false;
    for ch in value.chars() {
        if output.len().saturating_add(ch.len_utf8()) > max_bytes {
            truncated = true;
            break;
        }
        output.push(ch);
    }
    if truncated {
        let marker = if max_bytes >= "... [truncated]".len() {
            "... [truncated]"
        } else if max_bytes >= 3 {
            "..."
        } else {
            ""
        };
        while output.len().saturating_add(marker.len()) > max_bytes {
            output.pop();
        }
        output.push_str(marker);
    }
    output
}

pub(crate) fn sanitize_display_text(value: &str, max_chars: usize) -> String {
    let mut output = String::new();
    for (count, ch) in value.chars().enumerate() {
        if count >= max_chars {
            output.push_str("...");
            break;
        }
        if ch.is_control() && ch != '\n' && ch != '\t' {
            output.push(' ');
        } else {
            output.push(ch);
        }
    }
    output
}

pub(crate) fn sanitize_tool_text(value: &str, max_bytes: usize) -> String {
    let mut lines = Vec::new();
    for line in value.lines() {
        let lower = line.to_ascii_lowercase();
        if contains_sensitive_marker(&lower) {
            lines.push("[redacted sensitive output]".to_string());
        } else {
            lines.push(sanitize_display_text(line, MAX_TOOL_DETAIL_TEXT_BYTES));
        }
    }
    truncate_display_text(&lines.join("\n"), max_bytes)
}

pub(crate) fn sanitize_json_value(value: &Value, depth: usize) -> Value {
    if depth > 8 {
        return Value::String("[depth limited]".to_string());
    }
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let lower = key.to_ascii_lowercase();
                    let value = if contains_sensitive_key(&lower) {
                        Value::String("[redacted]".to_string())
                    } else {
                        sanitize_json_value(value, depth + 1)
                    };
                    (sanitize_display_text(key, 80), value)
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .take(128)
                .map(|value| sanitize_json_value(value, depth + 1))
                .collect(),
        ),
        Value::String(text) => Value::String(sanitize_tool_text(text, MAX_TOOL_DETAIL_TEXT_BYTES)),
        other => other.clone(),
    }
}

fn contains_sensitive_key(lower: &str) -> bool {
    [
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "authorization",
        "cookie",
        "private_key",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn contains_sensitive_marker(lower: &str) -> bool {
    [
        "password",
        "passwd",
        "api_key",
        "apikey",
        "authorization",
        "bearer ",
        "access_token",
        "refresh_token",
        "private_key",
        "secret",
        "chain of thought",
        "hidden reasoning",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::{sanitize_json_value, sanitize_tool_text, truncate_display_text};

    #[test]
    fn nested_sensitive_json_keys_are_redacted() {
        let value = serde_json::json!({
            "outer": {"credentials": {"api_token": "do-not-show"}},
            "items": [{"private_key": "hidden"}],
        });
        let rendered = sanitize_json_value(&value, 0).to_string();
        assert!(!rendered.contains("do-not-show"));
        assert!(!rendered.contains("hidden"));
        assert!(rendered.contains("[redacted]"));
    }

    #[test]
    fn common_secret_markers_hide_entire_output_lines() {
        for input in [
            "Authorization: Bearer top-secret",
            "password=top-secret",
            "access_token: top-secret",
            "private_key: top-secret",
        ] {
            let rendered = sanitize_tool_text(input, 1024);
            assert!(!rendered.contains("top-secret"), "rendered {rendered:?}");
            assert!(rendered.contains("redacted"));
        }
    }

    #[test]
    fn truncation_keeps_utf8_boundaries_and_is_bounded() {
        let rendered = truncate_display_text("界界界界界", 7);
        assert!(rendered.is_char_boundary(rendered.len()));
        assert!(rendered.len() <= 7);
    }
}
