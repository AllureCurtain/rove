//! A deliberately restricted frontmatter parser.
//!
//! Procedure documents carry machine-readable frontmatter between `---`
//! fences (design §11.2). The accepted grammar is a small subset of YAML:
//! `key: scalar` and `key: [a, b, c]`, one pair per line, no nesting.
//!
//! The restriction is the point. A general YAML parser brings anchors,
//! aliases, merge keys, tags, and implicit type coercion — all of which are
//! ways for an untrusted document to surprise the reader. Procedure documents
//! come from outside the runtime, so the parser accepts exactly the shapes the
//! schema needs and rejects everything else with a stable code.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Largest frontmatter block accepted, in bytes.
pub const MAX_FRONTMATTER_BYTES: usize = 8 * 1024;
/// Largest number of keys accepted in one frontmatter block.
pub const MAX_FRONTMATTER_KEYS: usize = 48;
/// Largest number of items accepted in one sequence value.
pub const MAX_SEQUENCE_ITEMS: usize = 32;
/// Largest single scalar value, in characters.
pub const MAX_SCALAR_CHARS: usize = 512;

/// One parsed frontmatter value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FrontmatterValue {
    Scalar(String),
    Sequence(Vec<String>),
}

impl FrontmatterValue {
    /// The scalar text, or `None` for a sequence.
    pub fn as_scalar(&self) -> Option<&str> {
        match self {
            Self::Scalar(value) => Some(value),
            Self::Sequence(_) => None,
        }
    }

    /// Items of a sequence. A scalar is *not* silently treated as a one-item
    /// sequence: `platforms: linux` and `platforms: [linux]` are different
    /// authoring mistakes and conflating them hides one of them.
    pub fn as_sequence(&self) -> Option<&[String]> {
        match self {
            Self::Sequence(items) => Some(items),
            Self::Scalar(_) => None,
        }
    }
}

/// A parsed frontmatter block plus the body that followed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitDocument {
    pub frontmatter: BTreeMap<String, FrontmatterValue>,
    pub body: String,
    /// Raw frontmatter text, retained for hashing.
    pub raw_frontmatter: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case", tag = "error")]
pub enum FrontmatterError {
    #[error("document does not start with a '---' frontmatter fence")]
    MissingOpeningFence,
    #[error("frontmatter is not terminated by a closing '---' fence")]
    MissingClosingFence,
    #[error("frontmatter is {len} bytes, over the {max} byte limit")]
    TooLarge { len: usize, max: usize },
    #[error("frontmatter has {count} keys, over the {max} key limit")]
    TooManyKeys { count: usize, max: usize },
    #[error("line {line}: expected 'key: value'")]
    MalformedLine { line: usize },
    #[error("line {line}: duplicate key '{key}'")]
    DuplicateKey { line: usize, key: String },
    #[error("line {line}: key '{key}' is not a valid identifier")]
    InvalidKey { line: usize, key: String },
    #[error("line {line}: sequence has {count} items, over the {max} item limit")]
    SequenceTooLong {
        line: usize,
        count: usize,
        max: usize,
    },
    #[error("line {line}: value is {len} characters, over the {max} character limit")]
    ScalarTooLong { line: usize, len: usize, max: usize },
    #[error("line {line}: nested or indented structures are not supported")]
    UnsupportedNesting { line: usize },
    #[error("line {line}: YAML anchors, aliases, and tags are not supported")]
    UnsupportedYamlFeature { line: usize },
}

impl FrontmatterError {
    /// Stable machine-readable code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingOpeningFence => "missing_opening_fence",
            Self::MissingClosingFence => "missing_closing_fence",
            Self::TooLarge { .. } => "frontmatter_too_large",
            Self::TooManyKeys { .. } => "too_many_keys",
            Self::MalformedLine { .. } => "malformed_line",
            Self::DuplicateKey { .. } => "duplicate_key",
            Self::InvalidKey { .. } => "invalid_key",
            Self::SequenceTooLong { .. } => "sequence_too_long",
            Self::ScalarTooLong { .. } => "scalar_too_long",
            Self::UnsupportedNesting { .. } => "unsupported_nesting",
            Self::UnsupportedYamlFeature { .. } => "unsupported_yaml_feature",
        }
    }
}

/// Split a document into frontmatter and body, then parse the frontmatter.
///
/// Frontmatter and body are parsed separately (design §21.1) so a body that
/// contains a `---` line cannot re-open the metadata block and inject a key.
pub fn split_document(text: &str) -> Result<SplitDocument, FrontmatterError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");

    let after_open = normalized
        .strip_prefix("---\n")
        .or_else(|| {
            normalized
                .strip_prefix("---")
                .and_then(|rest| rest.strip_prefix('\n'))
        })
        .ok_or(FrontmatterError::MissingOpeningFence)?;

    // Only a line that is exactly `---` closes the block. A `---` with
    // trailing content is not a fence, so it cannot end metadata early.
    let mut frontmatter_lines = Vec::new();
    let mut body_start = None;
    let mut consumed = 0usize;
    for line in after_open.split_inclusive('\n') {
        let trimmed = line.trim_end_matches('\n');
        consumed += line.len();
        if trimmed == "---" {
            body_start = Some(consumed);
            break;
        }
        frontmatter_lines.push(trimmed.to_string());
    }

    let Some(body_start) = body_start else {
        return Err(FrontmatterError::MissingClosingFence);
    };

    let raw_frontmatter = frontmatter_lines.join("\n");
    if raw_frontmatter.len() > MAX_FRONTMATTER_BYTES {
        return Err(FrontmatterError::TooLarge {
            len: raw_frontmatter.len(),
            max: MAX_FRONTMATTER_BYTES,
        });
    }

    let frontmatter = parse_frontmatter(&frontmatter_lines)?;
    let body = after_open[body_start..]
        .trim_start_matches('\n')
        .to_string();

    Ok(SplitDocument {
        frontmatter,
        body,
        raw_frontmatter,
    })
}

fn parse_frontmatter(
    lines: &[String],
) -> Result<BTreeMap<String, FrontmatterValue>, FrontmatterError> {
    let mut parsed = BTreeMap::new();

    for (index, line) in lines.iter().enumerate() {
        // Frontmatter line numbers are 1-based and start after the fence, so
        // an error points at the line the author actually sees.
        let line_number = index + 2;

        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        // Indentation implies nesting, which this grammar does not accept.
        if line.starts_with(' ') || line.starts_with('\t') {
            return Err(FrontmatterError::UnsupportedNesting { line: line_number });
        }
        if line.trim_start().starts_with('-') {
            return Err(FrontmatterError::UnsupportedNesting { line: line_number });
        }

        let Some((key, value)) = line.split_once(':') else {
            return Err(FrontmatterError::MalformedLine { line: line_number });
        };
        let key = key.trim();
        let value = value.trim();

        validate_key(key, line_number)?;
        reject_yaml_features(value, line_number)?;

        let parsed_value = if let Some(inner) = value.strip_prefix('[') {
            let inner = inner
                .strip_suffix(']')
                .ok_or(FrontmatterError::MalformedLine { line: line_number })?;
            parse_sequence(inner, line_number)?
        } else {
            let scalar = unquote(value);
            if scalar.chars().count() > MAX_SCALAR_CHARS {
                return Err(FrontmatterError::ScalarTooLong {
                    line: line_number,
                    len: scalar.chars().count(),
                    max: MAX_SCALAR_CHARS,
                });
            }
            FrontmatterValue::Scalar(scalar)
        };

        if parsed.contains_key(key) {
            return Err(FrontmatterError::DuplicateKey {
                line: line_number,
                key: key.to_string(),
            });
        }
        if parsed.len() >= MAX_FRONTMATTER_KEYS {
            return Err(FrontmatterError::TooManyKeys {
                count: parsed.len() + 1,
                max: MAX_FRONTMATTER_KEYS,
            });
        }
        parsed.insert(key.to_string(), parsed_value);
    }

    Ok(parsed)
}

fn validate_key(key: &str, line: usize) -> Result<(), FrontmatterError> {
    let valid = !key.is_empty()
        && key.len() <= 64
        && key.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        });
    if valid {
        Ok(())
    } else {
        Err(FrontmatterError::InvalidKey {
            line,
            key: key.chars().filter(|c| !c.is_control()).take(48).collect(),
        })
    }
}

/// Reject the YAML features this grammar deliberately does not implement.
///
/// Accepting them silently would be worse than rejecting them: an author
/// would believe an anchor or tag took effect when the value was actually
/// read as literal text.
fn reject_yaml_features(value: &str, line: usize) -> Result<(), FrontmatterError> {
    if value.starts_with('&')
        || value.starts_with('*')
        || value.starts_with("!!")
        || value.starts_with('!')
    {
        return Err(FrontmatterError::UnsupportedYamlFeature { line });
    }
    if value == "|" || value == ">" || value.starts_with("|-") || value.starts_with(">-") {
        return Err(FrontmatterError::UnsupportedNesting { line });
    }
    if value.starts_with('{') {
        return Err(FrontmatterError::UnsupportedNesting { line });
    }
    Ok(())
}

fn parse_sequence(inner: &str, line: usize) -> Result<FrontmatterValue, FrontmatterError> {
    let trimmed = inner.trim();
    if trimmed.is_empty() {
        return Ok(FrontmatterValue::Sequence(Vec::new()));
    }

    let mut items = Vec::new();
    for raw in trimmed.split(',') {
        let item = unquote(raw.trim());
        if item.chars().count() > MAX_SCALAR_CHARS {
            return Err(FrontmatterError::ScalarTooLong {
                line,
                len: item.chars().count(),
                max: MAX_SCALAR_CHARS,
            });
        }
        if items.len() >= MAX_SEQUENCE_ITEMS {
            return Err(FrontmatterError::SequenceTooLong {
                line,
                count: items.len() + 1,
                max: MAX_SEQUENCE_ITEMS,
            });
        }
        items.push(item);
    }
    Ok(FrontmatterValue::Sequence(items))
}

/// Strip one layer of matching quotes and drop control characters.
fn unquote(value: &str) -> String {
    let unquoted = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|rest| rest.strip_suffix('\''))
        })
        .unwrap_or(value);
    unquoted
        .chars()
        .filter(|character| !character.is_control() || *character == '\t')
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars_and_sequences_parse() {
        let document = split_document(
            "---\nid: ops.disk\nversion: 1.0.0\ntags: [ops, disk, incident]\nempty_list: []\n---\n\n# When to use\n\nBody.\n",
        )
        .unwrap();

        assert_eq!(
            document.frontmatter.get("id").unwrap().as_scalar(),
            Some("ops.disk")
        );
        assert_eq!(
            document.frontmatter.get("tags").unwrap().as_sequence(),
            Some(
                [
                    "ops".to_string(),
                    "disk".to_string(),
                    "incident".to_string()
                ]
                .as_slice()
            )
        );
        assert_eq!(
            document
                .frontmatter
                .get("empty_list")
                .unwrap()
                .as_sequence(),
            Some([].as_slice())
        );
        assert!(document.body.starts_with("# When to use"));
    }

    #[test]
    fn crlf_documents_parse_identically() {
        let unix = split_document("---\nid: a\n---\nbody\n").unwrap();
        let windows = split_document("---\r\nid: a\r\n---\r\nbody\r\n").unwrap();

        assert_eq!(unix.frontmatter, windows.frontmatter);
        assert_eq!(unix.body, windows.body);
    }

    #[test]
    fn a_missing_fence_is_reported_not_guessed() {
        assert_eq!(
            split_document("id: a\n"),
            Err(FrontmatterError::MissingOpeningFence)
        );
        assert_eq!(
            split_document("---\nid: a\n"),
            Err(FrontmatterError::MissingClosingFence)
        );
    }

    /// A `---` inside the body must not be able to re-open the metadata block
    /// and inject a key such as a higher trust level.
    #[test]
    fn a_body_fence_cannot_inject_frontmatter_keys() {
        let document = split_document(
            "---\nid: real\n---\n\n# Body\n\n---\nid: injected\ntrust: builtin_trusted\n---\n",
        )
        .unwrap();

        assert_eq!(
            document.frontmatter.get("id").unwrap().as_scalar(),
            Some("real")
        );
        assert!(
            !document.frontmatter.contains_key("trust"),
            "body content leaked into frontmatter: {:?}",
            document.frontmatter
        );
        assert!(document.body.contains("id: injected"));
    }

    #[test]
    fn yaml_anchors_aliases_and_tags_are_rejected() {
        for value in [
            "&anchor text",
            "*alias",
            "!!python/object:os.system",
            "!Tag x",
        ] {
            let error = split_document(&format!("---\nkey: {value}\n---\nbody\n")).unwrap_err();
            assert_eq!(
                error.code(),
                "unsupported_yaml_feature",
                "{value} produced {error:?}"
            );
        }
    }

    #[test]
    fn nesting_block_scalars_and_flow_maps_are_rejected() {
        for text in [
            "---\nkey:\n  nested: 1\n---\nbody\n",
            "---\nkey: |\n  block\n---\nbody\n",
            "---\nkey: {a: 1}\n---\nbody\n",
            "---\nlist:\n- item\n---\nbody\n",
        ] {
            let error = split_document(text).unwrap_err();
            assert_eq!(error.code(), "unsupported_nesting", "{text:?} -> {error:?}");
        }
    }

    #[test]
    fn duplicate_and_invalid_keys_are_rejected() {
        assert_eq!(
            split_document("---\nid: a\nid: b\n---\nbody\n")
                .unwrap_err()
                .code(),
            "duplicate_key"
        );
        for key in ["Id", "my-key", "key!", "", "a b"] {
            let error = split_document(&format!("---\n{key}: value\n---\nbody\n")).unwrap_err();
            assert!(
                matches!(error.code(), "invalid_key" | "malformed_line"),
                "{key:?} produced {error:?}"
            );
        }
    }

    #[test]
    fn a_line_without_a_colon_is_malformed() {
        assert_eq!(
            split_document("---\nnot a pair\n---\nbody\n")
                .unwrap_err()
                .code(),
            "malformed_line"
        );
    }

    #[test]
    fn all_limits_are_enforced() {
        let long_scalar = "x".repeat(MAX_SCALAR_CHARS + 1);
        assert_eq!(
            split_document(&format!("---\nkey: {long_scalar}\n---\nbody\n"))
                .unwrap_err()
                .code(),
            "scalar_too_long"
        );

        let many_items = (0..=MAX_SEQUENCE_ITEMS)
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        assert_eq!(
            split_document(&format!("---\nkey: [{many_items}]\n---\nbody\n"))
                .unwrap_err()
                .code(),
            "sequence_too_long"
        );

        let many_keys = (0..=MAX_FRONTMATTER_KEYS)
            .map(|index| format!("key_{index}: value"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            split_document(&format!("---\n{many_keys}\n---\nbody\n"))
                .unwrap_err()
                .code(),
            "too_many_keys"
        );

        let huge = format!("key: {}", "y".repeat(MAX_FRONTMATTER_BYTES + 10));
        assert_eq!(
            split_document(&format!("---\n{huge}\n---\nbody\n"))
                .unwrap_err()
                .code(),
            "frontmatter_too_large"
        );
    }

    #[test]
    fn quotes_are_stripped_and_control_characters_dropped() {
        let document = split_document(
            "---\nsummary: \"quoted value\"\ntitle: 'single'\nsneaky: a\u{7}b\n---\nbody\n",
        )
        .unwrap();

        assert_eq!(
            document.frontmatter.get("summary").unwrap().as_scalar(),
            Some("quoted value")
        );
        assert_eq!(
            document.frontmatter.get("title").unwrap().as_scalar(),
            Some("single")
        );
        assert_eq!(
            document.frontmatter.get("sneaky").unwrap().as_scalar(),
            Some("ab"),
            "control characters must not survive into metadata"
        );
    }

    /// A scalar is not silently promoted to a sequence: that would hide an
    /// authoring mistake in a field the filter depends on.
    #[test]
    fn a_scalar_is_not_treated_as_a_one_item_sequence() {
        let document = split_document("---\nplatforms: linux\n---\nbody\n").unwrap();
        let value = document.frontmatter.get("platforms").unwrap();

        assert_eq!(value.as_scalar(), Some("linux"));
        assert_eq!(value.as_sequence(), None);
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let document =
            split_document("---\n# a comment\n\nid: a\n\n# another\n---\nbody\n").unwrap();

        assert_eq!(document.frontmatter.len(), 1);
        assert_eq!(
            document.frontmatter.get("id").unwrap().as_scalar(),
            Some("a")
        );
    }

    #[test]
    fn a_fence_with_trailing_content_does_not_close_the_block() {
        // `--- not a fence` is a malformed line, not a terminator, so the
        // block runs to the real fence below it.
        let document = split_document("---\nid: a\n--- trailing\n---\nbody\n");
        assert!(
            document.is_err(),
            "expected malformed line, got {document:?}"
        );
    }

    #[test]
    fn a_bom_prefixed_document_still_parses() {
        let document = split_document("\u{feff}---\nid: a\n---\nbody\n").unwrap();
        assert_eq!(
            document.frontmatter.get("id").unwrap().as_scalar(),
            Some("a")
        );
    }
}
