//! Locating a hunk's expected context inside a file.
//!
//! Model-authored patches routinely disagree with the file in ways that do not
//! change meaning: re-indented lines, trailing whitespace, or tabs versus
//! spaces. Matching therefore proceeds in widening passes and reports which
//! pass succeeded, so callers can treat a shaky match differently from an exact
//! one instead of silently accepting either.
//!
//! Every function here is pure and allocation-light; no IO, no async.

/// How confident a located match is.
///
/// Ordering is meaningful: `Exact` is the strongest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchConfidence {
    /// Every line matched byte for byte.
    Exact,
    /// Lines matched after ignoring trailing whitespace.
    TrailingWhitespace,
    /// Lines matched after normalizing all leading/interior whitespace runs.
    Whitespace,
}

impl MatchConfidence {
    /// Whether this match is weak enough that callers should surface it.
    ///
    /// Exact matches need no explanation; anything looser means the patch and
    /// the file disagreed on whitespace and the caller may want to report it.
    pub fn is_fuzzy(self) -> bool {
        self != Self::Exact
    }

    /// Human-readable label for diagnostics.
    pub fn label(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::TrailingWhitespace => "trailing-whitespace-insensitive",
            Self::Whitespace => "whitespace-insensitive",
        }
    }
}

/// A successful context location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchOutcome {
    /// Index of the first matched line in the haystack.
    pub start: usize,
    /// Number of haystack lines the match spans.
    pub length: usize,
    pub confidence: MatchConfidence,
}

/// Find `needle` within `haystack`, widening tolerance until something matches.
///
/// Returns `None` when no pass locates the context, and `Err`-like ambiguity is
/// signalled by [`locate_context_unique`]. An empty needle matches at `hint` (or
/// 0) with zero length, which lets pure insertions anchor without context.
///
/// `hint` biases the search: candidates at or after the hint are preferred, so a
/// patch whose hunks appear in file order does not re-match an earlier
/// occurrence.
pub fn locate_context(haystack: &[&str], needle: &[&str], hint: usize) -> Option<MatchOutcome> {
    if needle.is_empty() {
        return Some(MatchOutcome {
            start: hint.min(haystack.len()),
            length: 0,
            confidence: MatchConfidence::Exact,
        });
    }
    for confidence in [
        MatchConfidence::Exact,
        MatchConfidence::TrailingWhitespace,
        MatchConfidence::Whitespace,
    ] {
        if let Some(start) = search(haystack, needle, hint, confidence) {
            return Some(MatchOutcome {
                start,
                length: needle.len(),
                confidence,
            });
        }
    }
    None
}

/// Count how many positions match `needle` at the strongest confidence that
/// yields any match. Used to detect ambiguous context.
pub(crate) fn count_matches(haystack: &[&str], needle: &[&str]) -> usize {
    if needle.is_empty() {
        return 1;
    }
    for confidence in [
        MatchConfidence::Exact,
        MatchConfidence::TrailingWhitespace,
        MatchConfidence::Whitespace,
    ] {
        let count = (0..=haystack.len().saturating_sub(needle.len()))
            .filter(|&offset| window_matches(haystack, needle, offset, confidence))
            .count();
        if count > 0 {
            return count;
        }
    }
    0
}

/// Scan for the first match at `confidence`, preferring positions >= `hint`.
fn search(
    haystack: &[&str],
    needle: &[&str],
    hint: usize,
    confidence: MatchConfidence,
) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    let last = haystack.len() - needle.len();
    let start_at = hint.min(last);
    for offset in start_at..=last {
        if window_matches(haystack, needle, offset, confidence) {
            return Some(offset);
        }
    }
    // Fall back to positions before the hint so out-of-order hunks still apply.
    (0..start_at).find(|offset| window_matches(haystack, needle, *offset, confidence))
}

fn window_matches(
    haystack: &[&str],
    needle: &[&str],
    offset: usize,
    confidence: MatchConfidence,
) -> bool {
    needle
        .iter()
        .enumerate()
        .all(|(index, expected)| lines_equal(haystack[offset + index], expected, confidence))
}

fn lines_equal(actual: &str, expected: &str, confidence: MatchConfidence) -> bool {
    match confidence {
        MatchConfidence::Exact => actual == expected,
        MatchConfidence::TrailingWhitespace => actual.trim_end() == expected.trim_end(),
        MatchConfidence::Whitespace => {
            normalize_whitespace(actual) == normalize_whitespace(expected)
        }
    }
}

/// Collapse every whitespace run to a single space and trim the ends.
///
/// This is what makes re-indentation and tab/space drift tolerable. It operates
/// on `char`s, so multi-byte content is never split mid-character.
fn normalize_whitespace(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_space = false;
    for ch in line.chars() {
        if ch.is_whitespace() {
            in_space = true;
            continue;
        }
        if in_space && !out.is_empty() {
            out.push(' ');
        }
        in_space = false;
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_wins_over_fuzzy_candidates() {
        let haystack = ["fn a() {", "    let x = 1;", "}"];
        let needle = ["    let x = 1;"];
        let found = locate_context(&haystack, &needle, 0).unwrap();
        assert_eq!(found.start, 1);
        assert_eq!(found.confidence, MatchConfidence::Exact);
        assert!(!found.confidence.is_fuzzy());
    }

    #[test]
    fn reindented_context_matches_at_whitespace_confidence() {
        let haystack = ["fn a() {", "\t\tlet x = 1;", "}"];
        let needle = ["    let x = 1;"];
        let found = locate_context(&haystack, &needle, 0).unwrap();
        assert_eq!(found.start, 1);
        assert_eq!(found.confidence, MatchConfidence::Whitespace);
        assert!(found.confidence.is_fuzzy());
    }

    #[test]
    fn trailing_whitespace_is_a_weaker_match_than_exact_but_stronger_than_reindent() {
        let haystack = ["let x = 1;   "];
        let needle = ["let x = 1;"];
        let found = locate_context(&haystack, &needle, 0).unwrap();
        assert_eq!(found.confidence, MatchConfidence::TrailingWhitespace);
        assert!(MatchConfidence::Exact < MatchConfidence::TrailingWhitespace);
        assert!(MatchConfidence::TrailingWhitespace < MatchConfidence::Whitespace);
    }

    #[test]
    fn unmatched_context_is_reported_as_none() {
        let haystack = ["fn a() {}"];
        let needle = ["fn b() {}"];
        assert!(locate_context(&haystack, &needle, 0).is_none());
    }

    #[test]
    fn the_hint_biases_toward_later_occurrences() {
        let haystack = ["dup", "mid", "dup"];
        let needle = ["dup"];
        assert_eq!(locate_context(&haystack, &needle, 0).unwrap().start, 0);
        assert_eq!(locate_context(&haystack, &needle, 1).unwrap().start, 2);
    }

    #[test]
    fn a_hint_past_the_end_still_finds_an_earlier_match() {
        let haystack = ["only"];
        let needle = ["only"];
        assert_eq!(locate_context(&haystack, &needle, 99).unwrap().start, 0);
    }

    #[test]
    fn an_empty_needle_anchors_at_the_hint_without_consuming_lines() {
        let haystack = ["a", "b"];
        let found = locate_context(&haystack, &[], 1).unwrap();
        assert_eq!((found.start, found.length), (1, 0));
    }

    #[test]
    fn ambiguity_is_counted_at_the_strongest_matching_confidence() {
        // Two exact matches: ambiguous.
        assert_eq!(count_matches(&["dup", "x", "dup"], &["dup"]), 2);
        // One exact match plus a whitespace-only variant: the exact pass wins
        // and reports a single match, so this is not treated as ambiguous.
        assert_eq!(count_matches(&["dup", "  dup  "], &["dup"]), 1);
    }

    #[test]
    fn whitespace_normalization_never_splits_multibyte_characters() {
        let haystack = ["  日本語\tテスト  "];
        let needle = ["日本語 テスト"];
        let found = locate_context(&haystack, &needle, 0).unwrap();
        assert_eq!(found.confidence, MatchConfidence::Whitespace);
    }
}
