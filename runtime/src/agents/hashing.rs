//! Canonical content hashing for definitions, instructions, and procedures.
//!
//! Design §8.2 requires that changing only line endings or serialization order
//! must not change a hash. Without that rule a Windows checkout and a Linux
//! checkout of the same file would look like two different pinned snapshots,
//! and every resume across platforms would report false drift.
//!
//! Canonicalization is deliberately narrow: it normalizes representation, not
//! meaning. Interior blank lines, indentation, and letter case are preserved,
//! because those can carry meaning in Markdown instructions.

use serde::Serialize;

use crate::prompt_metadata::stable_hash;

/// Normalize text before hashing.
///
/// - CRLF and lone CR become LF, so the same bytes on Windows and Unix agree.
/// - Trailing spaces/tabs are dropped per line, which editors add invisibly.
/// - A trailing newline is normalized away, so "file ends with newline" and
///   "file does not" are the same content.
/// - A leading UTF-8 BOM is dropped; it is an encoding marker, not content.
pub fn canonicalize_text(text: &str) -> String {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut canonical = String::with_capacity(text.len());
    for (index, line) in text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .split('\n')
        .enumerate()
    {
        if index > 0 {
            canonical.push('\n');
        }
        canonical.push_str(line.trim_end_matches([' ', '\t']));
    }
    while canonical.ends_with('\n') {
        canonical.pop();
    }
    canonical
}

/// Hash canonicalized text with a domain tag.
///
/// The tag keeps hash spaces separate: an instruction file and a procedure
/// body with identical text must not collide into one identity, otherwise a
/// pinned procedure hash could be "satisfied" by an unrelated document.
pub fn content_hash(domain: &str, text: &str) -> String {
    stable_hash(&format!("{domain}\u{1f}{}", canonicalize_text(text)))
}

/// Hash a serializable value through its canonical JSON form.
///
/// `serde_json` emits struct fields in declaration order and `BTreeMap` keys
/// in sorted order, so a value built from differently ordered input still
/// produces one hash. Types that need order independence must therefore use
/// ordered collections; that is asserted in the definition tests.
pub fn structured_hash<T: Serialize>(domain: &str, value: &T) -> String {
    let encoded = serde_json::to_string(value).unwrap_or_default();
    stable_hash(&format!("{domain}\u{1f}{encoded}"))
}

/// Combine already-computed component hashes into one identity.
///
/// Input order is preserved by the caller when order is meaningful; callers
/// that need order independence sort first. Length-prefixing each component
/// prevents `["ab","c"]` and `["a","bc"]` from producing the same digest.
pub fn composite_hash(domain: &str, components: &[&str]) -> String {
    let mut encoded = String::from(domain);
    for component in components {
        encoded.push('\u{1f}');
        encoded.push_str(&component.len().to_string());
        encoded.push(':');
        encoded.push_str(component);
    }
    stable_hash(&encoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_endings_do_not_change_a_content_hash() {
        let unix = "# Title\n\nBody line\n";
        let windows = "# Title\r\n\r\nBody line\r\n";
        let old_mac = "# Title\r\rBody line\r";

        assert_eq!(content_hash("test", unix), content_hash("test", windows));
        assert_eq!(content_hash("test", unix), content_hash("test", old_mac));
    }

    #[test]
    fn trailing_whitespace_and_final_newline_do_not_change_a_hash() {
        assert_eq!(
            content_hash("test", "line one\nline two"),
            content_hash("test", "line one   \nline two\t\n\n")
        );
    }

    #[test]
    fn a_utf8_bom_does_not_change_a_hash() {
        assert_eq!(
            content_hash("test", "content"),
            content_hash("test", "\u{feff}content")
        );
    }

    /// Interior structure carries meaning in Markdown, so canonicalization
    /// must not collapse it.
    #[test]
    fn meaningful_differences_still_change_the_hash() {
        let base = content_hash("test", "# A\n\n## B\n");
        assert_ne!(base, content_hash("test", "# A\n## B\n"));
        assert_ne!(base, content_hash("test", "# a\n\n## B\n"));
        assert_ne!(base, content_hash("test", "# A\n\n  ## B\n"));
    }

    #[test]
    fn hash_domains_are_separated() {
        assert_ne!(
            content_hash("instruction", "same text"),
            content_hash("procedure", "same text")
        );
    }

    #[test]
    fn structured_hash_is_order_independent_for_sorted_maps() {
        use std::collections::BTreeMap;

        let mut first = BTreeMap::new();
        first.insert("b", 2);
        first.insert("a", 1);
        let mut second = BTreeMap::new();
        second.insert("a", 1);
        second.insert("b", 2);

        assert_eq!(
            structured_hash("test", &first),
            structured_hash("test", &second)
        );
    }

    #[test]
    fn composite_hash_is_not_ambiguous_across_component_boundaries() {
        assert_ne!(
            composite_hash("test", &["ab", "c"]),
            composite_hash("test", &["a", "bc"])
        );
        assert_eq!(
            composite_hash("test", &["ab", "c"]),
            composite_hash("test", &["ab", "c"])
        );
        assert_ne!(
            composite_hash("test", &["a"]),
            composite_hash("test", &["a", ""])
        );
    }

    #[test]
    fn hashes_are_namespaced_and_deterministic() {
        let hash = content_hash("test", "content");
        assert!(hash.starts_with("sha256:"));
        assert_eq!(hash, content_hash("test", "content"));
    }
}
