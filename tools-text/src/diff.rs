//! Rendering unified diffs for tool output.
//!
//! Extracted from `rove_runtime::tools::coding` so diff rendering is testable
//! without a workspace. The output format is unchanged: a `--- / +++` header,
//! one `@@` hunk narrowed to the changed region plus a few context lines, and a
//! byte-budget truncation marker.

use crate::MAX_DIFF_BYTES;

/// Context lines kept on each side of a change.
pub const DIFF_CONTEXT_LINES: usize = 3;

/// Render a diff for one file, narrowed to the changed region.
///
/// Identical inputs render as an empty string. Truncation is byte-bounded and
/// never splits a multi-byte character.
pub fn localized_diff(path: &str, before: &str, after: &str) -> String {
    render_unified_diff(path, before, after, DIFF_CONTEXT_LINES, MAX_DIFF_BYTES)
}

/// [`localized_diff`] with explicit context and byte budget, for callers that
/// need a tighter or looser rendering.
pub fn render_unified_diff(
    path: &str,
    before: &str,
    after: &str,
    context: usize,
    max_bytes: usize,
) -> String {
    if before == after {
        return String::new();
    }
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();

    let prefix = common_prefix_len(&before_lines, &after_lines);
    let suffix = common_suffix_len(&before_lines, &after_lines, prefix);

    let before_start = prefix.saturating_sub(context);
    let after_start = before_start;
    let before_end = before_lines
        .len()
        .saturating_sub(suffix)
        .saturating_add(context)
        .min(before_lines.len());
    let after_end = after_lines
        .len()
        .saturating_sub(suffix)
        .saturating_add(context)
        .min(after_lines.len());

    let mut diff = format!(
        "--- a/{path}\n+++ b/{path}\n@@ -{},{} +{},{} @@\n",
        before_start + 1,
        before_end.saturating_sub(before_start),
        after_start + 1,
        after_end.saturating_sub(after_start)
    );

    for line in &before_lines[before_start..prefix.min(before_end)] {
        push_diff_line(&mut diff, ' ', line);
    }
    for line in &before_lines[prefix..before_lines.len().saturating_sub(suffix)] {
        push_diff_line(&mut diff, '-', line);
        if diff.len() >= max_bytes {
            return truncate_utf8(diff, max_bytes, "\n... diff truncated\n");
        }
    }
    for line in &after_lines[prefix..after_lines.len().saturating_sub(suffix)] {
        push_diff_line(&mut diff, '+', line);
        if diff.len() >= max_bytes {
            return truncate_utf8(diff, max_bytes, "\n... diff truncated\n");
        }
    }
    let suffix_start = before_lines.len().saturating_sub(suffix);
    for line in &before_lines[suffix_start..before_end] {
        push_diff_line(&mut diff, ' ', line);
    }
    truncate_utf8(diff, max_bytes, "\n... diff truncated\n")
}

fn common_prefix_len(before: &[&str], after: &[&str]) -> usize {
    before
        .iter()
        .zip(after.iter())
        .take_while(|(left, right)| left == right)
        .count()
}

fn common_suffix_len(before: &[&str], after: &[&str], prefix: usize) -> usize {
    before[prefix..]
        .iter()
        .rev()
        .zip(after[prefix..].iter().rev())
        .take_while(|(left, right)| left == right)
        .count()
}

fn push_diff_line(diff: &mut String, prefix: char, line: &str) {
    diff.push(prefix);
    diff.push_str(line);
    diff.push('\n');
}

/// Truncate to a byte budget on a character boundary, appending `suffix`.
fn truncate_utf8(mut value: String, max_bytes: usize, suffix: &str) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    let target = max_bytes.saturating_sub(suffix.len());
    let mut end = target.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value.push_str(suffix);
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_content_renders_nothing() {
        assert!(localized_diff("a.txt", "same\n", "same\n").is_empty());
    }

    #[test]
    fn a_single_line_change_shows_one_removal_and_one_addition() {
        let diff = localized_diff("a.txt", "one\ntwo\nthree\n", "one\nTWO\nthree\n");
        assert!(diff.contains("--- a/a.txt"), "git-style prefix: {diff}");
        assert!(diff.contains("+++ b/a.txt"));
        assert!(diff.contains("-two"));
        assert!(diff.contains("+TWO"));
        assert!(diff.contains(" one"), "context is retained");
        assert!(diff.contains(" three"));
    }

    #[test]
    fn diff_is_narrowed_to_the_changed_region_not_the_whole_file() {
        let before: String = (0..500).map(|i| format!("line {i}\n")).collect();
        let after = before.replace("line 250\n", "CHANGED\n");
        let diff = localized_diff("big.txt", &before, &after);
        assert!(diff.contains("-line 250"));
        assert!(diff.contains("+CHANGED"));
        assert!(
            !diff.contains("line 1\n"),
            "distant lines must not be rendered"
        );
    }

    #[test]
    fn truncation_lands_on_a_character_boundary() {
        let before = String::new();
        // Each line is multi-byte, so a naive byte cut would split a char.
        let after: String = (0..4000).map(|_| "日本語のテキスト\n").collect();
        let diff = render_unified_diff("u.txt", &before, &after, 3, 512);
        assert!(diff.len() <= 512);
        assert!(diff.ends_with("... diff truncated\n"));
        // The invariant: the result is valid UTF-8 by construction. If a cut had
        // split a character, building this String would have panicked already.
        assert!(diff.is_char_boundary(diff.len()));
    }

    #[test]
    fn adding_to_an_empty_file_renders_only_additions() {
        let diff = localized_diff("new.txt", "", "first\nsecond\n");
        assert!(diff.contains("+first"));
        assert!(diff.contains("+second"));
        // Only the `@@` header may carry a `-`; no body line is a removal.
        assert!(
            !body_lines(&diff).any(|line| line.starts_with('-')),
            "nothing was removed: {diff}"
        );
    }

    #[test]
    fn deleting_all_content_renders_only_removals() {
        let diff = localized_diff("gone.txt", "first\nsecond\n", "");
        assert!(diff.contains("-first"));
        assert!(diff.contains("-second"));
        assert!(
            !body_lines(&diff).any(|line| line.starts_with('+')),
            "nothing was added: {diff}"
        );
    }

    /// Diff lines excluding the `--- / +++ / @@` header, so tests can assert on
    /// removals and additions without matching the header's own `-`/`+`.
    fn body_lines(diff: &str) -> impl Iterator<Item = &str> {
        diff.lines().skip_while(|line| {
            line.starts_with("---") || line.starts_with("+++") || line.starts_with("@@")
        })
    }
}
