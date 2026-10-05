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
    let visible = strip_reasoning_blocks(value);
    for line in visible.lines() {
        let lower = line.to_ascii_lowercase();
        if contains_sensitive_marker(&lower) || contains_token_shape(&lower) {
            lines.push("[redacted sensitive output]".to_string());
        } else {
            lines.push(sanitize_display_text(line, MAX_TOOL_DETAIL_TEXT_BYTES));
        }
    }
    truncate_display_text(&lines.join("\n"), max_bytes)
}

fn strip_reasoning_blocks(value: &str) -> String {
    let mut visible = value.to_string();
    for (open, close) in [
        ("<think>", "</think>"),
        ("<analysis>", "</analysis>"),
        ("<reasoning>", "</reasoning>"),
    ] {
        loop {
            let lower = visible.to_ascii_lowercase();
            let Some(start) = lower.find(open) else {
                break;
            };
            let content_start = start + open.len();
            let end = lower[content_start..]
                .find(close)
                .map(|relative| content_start + relative + close.len())
                .unwrap_or(visible.len());
            visible.replace_range(start..end, "");
        }
    }
    visible
        .lines()
        .filter(|line| {
            let lower = line.trim_start().to_ascii_lowercase();
            !["thought:", "reasoning:", "analysis:", "chain-of-thought:"]
                .iter()
                .any(|marker| lower.starts_with(marker))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn contains_token_shape(lower: &str) -> bool {
    lower
        .split(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    '"' | '\'' | ',' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}'
                )
        })
        .map(|token| token.trim_matches(|character: char| matches!(character, '.' | '=')))
        .any(|token| {
            ((token.starts_with("sk-") || token.starts_with("ghp_") || token.starts_with("xoxb-"))
                && token.len() > 8)
                || looks_like_jwt(token)
        })
}

fn looks_like_jwt(token: &str) -> bool {
    let segments = token.split('.').collect::<Vec<_>>();
    segments.len() == 3
        && segments.iter().all(|segment| {
            segment.len() >= 8
                && segment.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
                })
        })
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
        "credential",
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
        "password=",
        "password:",
        "passwd=",
        "passwd:",
        "api_key=",
        "api_key:",
        "api-key=",
        "api-key:",
        "apikey=",
        "apikey:",
        "authorization",
        "token=",
        "token:",
        "access_token=",
        "access_token:",
        "refresh_token=",
        "refresh_token:",
        "private_key",
        "secret=",
        "secret:",
        "chain of thought",
        "hidden reasoning",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
        || contains_bearer_token(lower)
}

fn contains_bearer_token(lower: &str) -> bool {
    let Some((_, suffix)) = lower.split_once("bearer ") else {
        return false;
    };
    let token = suffix
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_matches(|character: char| matches!(character, '"' | '\'' | ',' | ';'));
    token.len() >= 16
        && token.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
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
