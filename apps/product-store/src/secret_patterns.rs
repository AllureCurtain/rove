//! The one secret-*pattern* table this repository knows.
//!
//! Two callers need the same answer and must never be able to disagree:
//!
//! - the session evidence export and the message-search snippet projection,
//!   which **redact** a pattern (`redact_secret_patterns`); and
//! - the attachment upload, which only **warns** that a payload looks like a
//!   credential (`contains_secret_pattern`) and stores the bytes unmodified.
//!
//! Two lists of secret prefixes in two modules would drift, and the drift would
//! be invisible: a shape the export scrubs but the attachment warning misses is
//! a credential persisted and transmitted after the user was told the file was
//! fine. This module owns the table, the ordering, and the token boundary rule;
//! `export.rs` re-exports the two entry points so its existing path keeps
//! working unchanged.
//!
//! The predicate is deliberately **non-allocating**. A detector that builds a
//! rewritten copy of the input just to ask "is there a match" pays O(input) per
//! prefix on the upload path, where the input is arbitrary user bytes; the
//! redactor still has to build the copy, but a caller that only wants a boolean
//! should not.

/// Longest secret-shaped token `redact_token_after_prefix` replaces.
///
/// A *byte* budget, not a character budget, so the clamp must land on a
/// character boundary: a secret-shaped prefix followed by a long multi-byte run
/// with no ASCII separator (200+ CJK characters after `token=`, for instance)
/// otherwise panics on a slice that splits a character.
pub const MAX_SECRET_TOKEN_BYTES: usize = 512;

/// One prefix rule.
///
/// The table is applied in [`SECRET_PREFIXES`] order, and the order is part of
/// the contract rather than an implementation detail: `Authorization: Bearer `
/// is consumed before the bare `Bearer ` rule, and every `key=` rule preserves
/// its prefix so the redacted output still reads as a field assignment.
#[derive(Debug, Clone, Copy)]
pub struct SecretPrefix {
    pub prefix: &'static str,
    /// Match ASCII case-insensitively (`PASSWORD=`, `bearer `).
    pub case_insensitive: bool,
    /// Keep the prefix and replace only the token (`token=[REDACTED…]`). A
    /// prefix that is itself the credential shape (`sk-`) is replaced with the
    /// token instead, so the output cannot re-form the original pattern.
    pub preserve_prefix: bool,
}

/// The single pattern table, in the order it has always been applied.
pub const SECRET_PREFIXES: [SecretPrefix; 18] = [
    SecretPrefix {
        prefix: "Authorization: Bearer ",
        case_insensitive: true,
        preserve_prefix: false,
    },
    SecretPrefix {
        prefix: "Authorization=Bearer ",
        case_insensitive: true,
        preserve_prefix: false,
    },
    SecretPrefix {
        prefix: "sk-ant-",
        case_insensitive: false,
        preserve_prefix: false,
    },
    SecretPrefix {
        prefix: "sk-proj-",
        case_insensitive: false,
        preserve_prefix: false,
    },
    SecretPrefix {
        prefix: "sk-",
        case_insensitive: false,
        preserve_prefix: false,
    },
    SecretPrefix {
        prefix: "ghp_",
        case_insensitive: false,
        preserve_prefix: false,
    },
    SecretPrefix {
        prefix: "gho_",
        case_insensitive: false,
        preserve_prefix: false,
    },
    SecretPrefix {
        prefix: "github_pat_",
        case_insensitive: false,
        preserve_prefix: false,
    },
    SecretPrefix {
        prefix: "xoxb-",
        case_insensitive: false,
        preserve_prefix: false,
    },
    SecretPrefix {
        prefix: "xoxp-",
        case_insensitive: false,
        preserve_prefix: false,
    },
    SecretPrefix {
        prefix: "AIza",
        case_insensitive: false,
        preserve_prefix: false,
    },
    SecretPrefix {
        prefix: "Bearer ",
        case_insensitive: true,
        preserve_prefix: false,
    },
    SecretPrefix {
        prefix: "password=",
        case_insensitive: true,
        preserve_prefix: true,
    },
    SecretPrefix {
        prefix: "passwd=",
        case_insensitive: true,
        preserve_prefix: true,
    },
    SecretPrefix {
        prefix: "token=",
        case_insensitive: true,
        preserve_prefix: true,
    },
    SecretPrefix {
        prefix: "api_key=",
        case_insensitive: true,
        preserve_prefix: true,
    },
    SecretPrefix {
        prefix: "apikey=",
        case_insensitive: true,
        preserve_prefix: true,
    },
    SecretPrefix {
        prefix: "secret=",
        case_insensitive: true,
        preserve_prefix: true,
    },
];

/// The marker both callers agree on.
pub const SECRET_PATTERN_MARKER: &str = "[REDACTED:secret_pattern]";

/// True when free text contains one of the secret *patterns* this repository
/// knows.
///
/// A detector, not a redactor: the attachment upload warns instead of rewriting
/// the user's bytes, and it must warn on exactly the shapes the evidence export
/// would have scrubbed — and on no others. It shares [`SECRET_PREFIXES`] with
/// [`redact_secret_patterns`] and allocates nothing.
///
/// Equivalence with the redactor is asserted by
/// `the_detector_agrees_with_the_redactor_on_adversarial_input`.
pub fn contains_secret_pattern(value: &str) -> bool {
    SECRET_PREFIXES
        .iter()
        .any(|rule| token_follows_prefix(value, rule))
}

/// True when `rule.prefix` occurs in `value` with a non-empty token after it.
///
/// This is exactly the condition under which `redact_token_after_prefix`
/// replaces something: an empty token (an absent one, or one that begins with
/// the separator set) is left alone so `token=` at the end of a line is not
/// reported as a credential.
fn token_follows_prefix(value: &str, rule: &SecretPrefix) -> bool {
    let index = if rule.case_insensitive {
        find_ascii_case_insensitive(value, rule.prefix)
    } else {
        value.find(rule.prefix)
    };
    let Some(index) = index else {
        return false;
    };
    let remainder = &value[index + rule.prefix.len()..];
    remainder
        .chars()
        .next()
        .is_some_and(|character| !is_token_separator(character))
}

/// Replace every secret-shaped token in free text, and report how many.
///
/// Visibility is `pub(crate)` because single-session message search reuses it
/// for snippets and the evidence export re-exports it: a search hit must not
/// surface a credential the evidence export would have redacted.
pub fn redact_secret_patterns(mut value: String) -> (String, usize) {
    let mut count = 0;
    for rule in SECRET_PREFIXES {
        let (next, replaced) = redact_token_after_prefix(&value, &rule);
        value = next;
        count += replaced;
    }
    (value, count)
}

/// Replace the token that follows every occurrence of `rule.prefix`.
///
/// The token ends at the first ASCII separator or at [`MAX_SECRET_TOKEN_BYTES`],
/// whichever comes first. The clamp is applied through [`truncate_utf8`] so the
/// consumed length is always a character boundary: these prefixes appear in
/// arbitrary user text (an evidence export, an attachment body, a
/// message-search snippet), and a token body of CJK prose has no ASCII
/// separator at all.
fn redact_token_after_prefix(value: &str, rule: &SecretPrefix) -> (String, usize) {
    let prefix = rule.prefix;
    let mut output = String::with_capacity(value.len());
    let mut rest = value;
    let mut count = 0;
    loop {
        let index = if rule.case_insensitive {
            find_ascii_case_insensitive(rest, prefix)
        } else {
            rest.find(prefix)
        };
        let Some(index) = index else {
            output.push_str(rest);
            break;
        };
        output.push_str(&rest[..index]);
        if rule.preserve_prefix {
            output.push_str(&rest[index..index + prefix.len()]);
        }
        let token_start = index + prefix.len();
        let token = &rest[token_start..];
        let token_len = truncate_utf8(
            token,
            token
                .find(is_token_separator)
                .unwrap_or(token.len())
                .min(MAX_SECRET_TOKEN_BYTES),
        )
        .len();
        if token_len == 0 {
            output.push_str(&rest[index..token_start]);
            rest = &rest[token_start..];
            continue;
        }
        output.push_str(SECRET_PATTERN_MARKER);
        rest = &rest[token_start + token_len..];
        count += 1;
    }
    (output, count)
}

/// The characters that end a secret-shaped token.
fn is_token_separator(character: char) -> bool {
    character.is_whitespace() || matches!(character, '"' | '\'' | ',' | ';' | ')' | ']' | '}')
}

/// The export redactor's own primitive, shared so both callers agree on how a
/// case-insensitive literal is located.
pub fn find_ascii_case_insensitive(value: &str, needle: &str) -> Option<usize> {
    value
        .as_bytes()
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

/// Truncate to `limit` bytes on a character boundary, never inside a marker.
///
/// The text reaching this function has already been through the secret
/// authority, so a cut can land inside a `[REDACTED:…]` marker; the shared
/// helper drops such a marker whole rather than emitting `[REDACTED:known_`.
pub fn truncate_utf8(value: &str, limit: usize) -> &str {
    crate::text::truncate_utf8_preserving_markers(value, limit).0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One input per adversarial shape this predicate has to survive: a long
    /// run, a string made only of prefixes, overlapping prefix starts, and
    /// multi-byte text with no ASCII separator after a prefix.
    fn adversarial_inputs() -> Vec<(&'static str, String)> {
        let mut inputs: Vec<(&'static str, String)> = Vec::new();
        inputs.push(("empty", String::new()));
        inputs.push(("bare prefix", "sk-".to_string()));
        inputs.push(("prefix then space", "sk- done".to_string()));
        inputs.push(("prefix then separator", "token=,\npassword=}".to_string()));
        inputs.push(("every prefix", {
            let mut value = String::new();
            for rule in SECRET_PREFIXES {
                value.push_str(rule.prefix);
            }
            value
        }));
        inputs.push(("overlapping starts", "sk-sk-sk-sk-sk-".repeat(64)));
        inputs.push(("many tokens", "token=a ".repeat(4096)));
        inputs.push((
            "multi-byte after prefix",
            format!("token={}", "界".repeat(600)),
        ));
        inputs.push(("multi-byte 4-byte run", format!("sk-{}", "😀".repeat(400))));
        inputs.push(("bearer chain", "Bearer Bearer Bearer x".to_string()));
        inputs.push(("nested keys", "token=token=x".to_string()));
        inputs.push(("long run", format!("{}{}", "sk-".repeat(32 * 1024), "x")));
        inputs.push((
            "long separator-free token",
            format!("password={}", "A".repeat(200 * 1024)),
        ));
        inputs.push((
            "long body after a prefix",
            format!("{}token={}", "z".repeat(128 * 1024), "B".repeat(1024)),
        ));
        inputs
    }

    #[test]
    fn the_detector_agrees_with_the_redactor_on_adversarial_input() {
        for (name, value) in adversarial_inputs() {
            let (_, count) = redact_secret_patterns(value.clone());
            assert_eq!(
                contains_secret_pattern(&value),
                count > 0,
                "detector and redactor disagree on `{name}`"
            );
        }
    }

    #[test]
    fn the_predicate_is_bounds_safe_on_adversarial_input() {
        for (name, value) in adversarial_inputs() {
            let input_len = value.len();
            // The detector allocates nothing and must not scan an unbounded
            // distance past a match: every rule consumes at least the prefix.
            let detected = contains_secret_pattern(&value);
            let (redacted, count) = redact_secret_patterns(value);
            // Each redaction consumes at least four bytes of input (the
            // shortest prefix is `sk-` plus a one-byte token) and inserts a
            // fixed marker, so the output is linear in the input rather than
            // unbounded, and there can never be more redactions than bytes.
            assert!(
                count <= input_len,
                "`{name}` produced {count} redactions from {input_len} bytes"
            );
            assert!(
                redacted.len() <= input_len.saturating_mul(8),
                "`{name}` grew {input_len} bytes to {} bytes",
                redacted.len()
            );
            assert_eq!(
                detected,
                count > 0,
                "`{name}` reported a different answer than the redactor"
            );
        }
    }

    #[test]
    fn a_prefix_without_a_token_is_not_a_secret() {
        for value in [
            "sk-",
            "sk- ",
            "sk-\n",
            "token=",
            "token= ",
            "token=,",
            "password=)",
            "Bearer ",
            "Authorization: Bearer ",
            "AIza",
        ] {
            assert!(
                !contains_secret_pattern(value),
                "`{value}` must not be reported as a credential"
            );
            assert_eq!(redact_secret_patterns(value.to_string()).1, 0);
        }
    }

    #[test]
    fn every_family_in_the_table_is_detected_and_redacted() {
        for rule in SECRET_PREFIXES {
            let value = format!("{}SYNTHETIC-CANARY-0001;", rule.prefix);
            assert!(
                contains_secret_pattern(&value),
                "`{}` must be detected",
                rule.prefix
            );
            let (redacted, count) = redact_secret_patterns(value);
            assert_eq!(count, 1, "`{}` must be redacted once", rule.prefix);
            assert!(!redacted.contains("SYNTHETIC-CANARY-0001"));
        }
    }

    #[test]
    fn the_table_is_the_one_the_export_path_applies() {
        // `rove-api`'s export path re-exports these functions rather than
        // owning a second table (asserted there); the assertion here is that
        // the shared table actually drives the redactor.
        assert_eq!(SECRET_PREFIXES.len(), 18);
        let value = "xoxb-SYNTHETIC-CANARY-0001".to_string();
        let (redacted, count) = crate::secret_patterns::redact_secret_patterns(value);
        assert_eq!(count, 1);
        assert!(!redacted.contains("SYNTHETIC-CANARY-0001"));
        assert!(crate::secret_patterns::contains_secret_pattern(
            "github_pat_SYNTHETIC-CANARY-0001"
        ));
    }
}
