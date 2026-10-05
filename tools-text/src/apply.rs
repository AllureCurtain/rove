//! The pure apply kernel: `(input_files, patch) -> Result<ApplyOutcome, ApplyError>`.
//!
//! No IO happens here. Callers supply the current content of every path the
//! patch touches and receive the intended new content; enforcing workspace
//! boundaries, capabilities, and durability stays with the caller.
//!
//! Line endings are preserved per file: if the input used CRLF throughout, the
//! output does too, so applying a patch on Windows does not rewrite every line
//! of the file. This is load-bearing for rove, which declares Windows support.

use std::collections::BTreeMap;

use crate::matching::{count_matches, locate_context};
use crate::patch::{FileOperation, Hunk, Patch};

/// What a successful apply produced.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ApplyOutcome {
    /// One entry per file the patch changed, in patch order.
    pub changes: Vec<FileChange>,
    /// Human-readable notes about weak matches, for surfacing to the caller.
    /// Empty when every hunk matched exactly.
    pub warnings: Vec<String>,
}

impl ApplyOutcome {
    /// Whether any hunk needed whitespace tolerance to land.
    pub fn had_fuzzy_matches(&self) -> bool {
        !self.warnings.is_empty()
    }
}

/// One file's resulting content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    /// Set when the operation renames the file.
    pub move_to: Option<String>,
    pub kind: FileChangeKind,
    /// Content before the change; `None` for a newly added file.
    pub before: Option<String>,
    /// Content after the change; `None` when the file is deleted.
    pub after: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileChangeKind {
    Add,
    Update,
    Delete,
}

/// Why a patch could not be applied.
///
/// [`Self::is_retryable`] distinguishes "the model can fix this by re-reading
/// and re-emitting" from "this request cannot succeed as written".
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApplyError {
    #[error("`{path}` was not supplied to the apply kernel")]
    MissingInput { path: String },
    #[error("`{path}` already exists, so it cannot be added")]
    AlreadyExists { path: String },
    #[error("could not locate hunk {hunk} context in `{path}`")]
    ContextNotFound { path: String, hunk: usize },
    #[error("hunk {hunk} context occurs {occurrences} times in `{path}`; make it unique")]
    AmbiguousContext {
        path: String,
        hunk: usize,
        occurrences: usize,
    },
    #[error("hunk {hunk} in `{path}` overlaps an earlier hunk in the same patch")]
    OverlappingHunks { path: String, hunk: usize },
    #[error("`{path}` is not valid UTF-8 text")]
    NotText { path: String },
    #[error("patch touches `{path}` more than once")]
    DuplicatePath { path: String },
}

impl ApplyError {
    /// Whether re-reading the file and re-emitting the patch could succeed.
    ///
    /// Context that could not be found or was ambiguous is a patch-authoring
    /// problem the model can correct. A missing input, a non-text file, an
    /// add-over-existing, or a self-conflicting patch are structural: retrying
    /// the same request cannot help.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::ContextNotFound { .. } | Self::AmbiguousContext { .. }
        )
    }

    /// The path this failure concerns.
    pub fn path(&self) -> &str {
        match self {
            Self::MissingInput { path }
            | Self::AlreadyExists { path }
            | Self::ContextNotFound { path, .. }
            | Self::AmbiguousContext { path, .. }
            | Self::OverlappingHunks { path, .. }
            | Self::NotText { path }
            | Self::DuplicatePath { path } => path,
        }
    }
}

/// Apply `patch` against `input_files`, returning the intended new content.
///
/// `input_files` must contain an entry for every path the patch updates or
/// deletes; added paths must be absent. Nothing is written — the caller decides
/// whether to persist [`ApplyOutcome::changes`].
///
/// The whole patch is validated before any change is reported: a failure on the
/// last file means no change is returned at all, so callers cannot half-apply.
pub fn apply_patch(
    input_files: &BTreeMap<String, String>,
    patch: &Patch,
) -> Result<ApplyOutcome, ApplyError> {
    let mut outcome = ApplyOutcome::default();
    let mut seen: Vec<&str> = Vec::new();

    for operation in &patch.operations {
        let path = operation.path();
        if seen.contains(&path) {
            return Err(ApplyError::DuplicatePath {
                path: path.to_string(),
            });
        }
        seen.push(path);

        match operation {
            FileOperation::Add { path, content } => {
                if input_files.contains_key(path) {
                    return Err(ApplyError::AlreadyExists { path: path.clone() });
                }
                outcome.changes.push(FileChange {
                    path: path.clone(),
                    move_to: None,
                    kind: FileChangeKind::Add,
                    before: None,
                    after: Some(content.clone()),
                });
            }
            FileOperation::Delete { path } => {
                let before = input_files
                    .get(path)
                    .ok_or_else(|| ApplyError::MissingInput { path: path.clone() })?;
                outcome.changes.push(FileChange {
                    path: path.clone(),
                    move_to: None,
                    kind: FileChangeKind::Delete,
                    before: Some(before.clone()),
                    after: None,
                });
            }
            FileOperation::Update {
                path,
                move_to,
                hunks,
            } => {
                let before = input_files
                    .get(path)
                    .ok_or_else(|| ApplyError::MissingInput { path: path.clone() })?;
                let (after, mut warnings) = apply_hunks(path, before, hunks)?;
                outcome.warnings.append(&mut warnings);
                outcome.changes.push(FileChange {
                    path: path.clone(),
                    move_to: move_to.clone(),
                    kind: FileChangeKind::Update,
                    before: Some(before.clone()),
                    after: Some(after),
                });
            }
        }
    }
    Ok(outcome)
}

/// Apply every hunk to one file's content, preserving its line-ending style.
fn apply_hunks(
    path: &str,
    before: &str,
    hunks: &[Hunk],
) -> Result<(String, Vec<String>), ApplyError> {
    let style = EndingStyle::detect(before);
    let mut lines: Vec<String> = split_lines(before);
    let mut warnings = Vec::new();
    // Regions already rewritten by earlier hunks, as (start, end) over the
    // current `lines`. Later hunks may not touch them.
    let mut claimed: Vec<(usize, usize)> = Vec::new();
    let mut cursor = 0usize;

    for (index, hunk) in hunks.iter().enumerate() {
        let hunk_number = index + 1;
        let expected = hunk.expected_lines();
        let view: Vec<&str> = lines.iter().map(String::as_str).collect();

        if !expected.is_empty() {
            let occurrences = count_matches(&view, &expected);
            if occurrences == 0 {
                return Err(ApplyError::ContextNotFound {
                    path: path.to_string(),
                    hunk: hunk_number,
                });
            }
            if occurrences > 1 && hunk.heading.is_none() {
                return Err(ApplyError::AmbiguousContext {
                    path: path.to_string(),
                    hunk: hunk_number,
                    occurrences,
                });
            }
        }

        // A heading narrows the search window: start looking after it.
        let search_hint = match hunk.heading.as_deref() {
            Some(heading) => locate_context(&view, &[heading], cursor)
                .map(|found| found.start + 1)
                .unwrap_or(cursor),
            None => cursor,
        };

        let found = locate_context(&view, &expected, search_hint).ok_or_else(|| {
            ApplyError::ContextNotFound {
                path: path.to_string(),
                hunk: hunk_number,
            }
        })?;
        let start = found.start;
        let end = start + found.length;

        if claimed
            .iter()
            .any(|&(claimed_start, claimed_end)| start < claimed_end && claimed_start < end)
        {
            return Err(ApplyError::OverlappingHunks {
                path: path.to_string(),
                hunk: hunk_number,
            });
        }

        if found.confidence.is_fuzzy() {
            warnings.push(format!(
                "{path}: hunk {hunk_number} matched at line {} using {} comparison",
                start + 1,
                found.confidence.label()
            ));
        }

        let replacement: Vec<String> = hunk
            .replacement_lines()
            .into_iter()
            .map(str::to_string)
            .collect();
        let replaced_len = replacement.len();
        lines.splice(start..end, replacement);

        // Shift previously claimed regions that sit after this edit.
        let delta = replaced_len as isize - (end - start) as isize;
        for region in claimed.iter_mut() {
            if region.0 >= end {
                region.0 = (region.0 as isize + delta) as usize;
                region.1 = (region.1 as isize + delta) as usize;
            }
        }
        claimed.push((start, start + replaced_len));
        cursor = start + replaced_len;
    }

    Ok((style.join(&lines, before), warnings))
}

/// Replace exactly one occurrence of `old_text` with `new_text`.
///
/// This is the pure core of rove's `edit_file` tool: the uniqueness requirement
/// is what makes an exact edit safe without a version check. Returns `None` when
/// `old_text` does not occur exactly once, which the caller reports as invalid
/// input.
pub fn replace_once(before: &str, old_text: &str, new_text: &str) -> Option<String> {
    if old_text.is_empty() {
        return None;
    }
    let mut matches = before.match_indices(old_text);
    let (index, _) = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    let mut after = String::with_capacity(before.len() + new_text.len());
    after.push_str(&before[..index]);
    after.push_str(new_text);
    after.push_str(&before[index + old_text.len()..]);
    Some(after)
}

/// Which line terminator a file uses, so edits do not rewrite untouched lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndingStyle {
    Lf,
    Crlf,
}

impl EndingStyle {
    /// CRLF only when the file uses it consistently; a mixed file is treated as
    /// LF so an edit does not spread CRLF into LF-only regions.
    fn detect(content: &str) -> Self {
        let total = content.matches('\n').count();
        if total == 0 {
            return Self::Lf;
        }
        let crlf = content.matches("\r\n").count();
        if crlf == total { Self::Crlf } else { Self::Lf }
    }

    fn join(self, lines: &[String], original: &str) -> String {
        let terminator = match self {
            Self::Lf => "\n",
            Self::Crlf => "\r\n",
        };
        let mut out = lines.join(terminator);
        // Preserve whether the file ended with a newline.
        if original.ends_with('\n') && !out.is_empty() {
            out.push_str(terminator);
        }
        out
    }
}

/// Split into logical lines, dropping the terminators (recorded by
/// [`EndingStyle`]) and the trailing empty element a final newline produces.
fn split_lines(content: &str) -> Vec<String> {
    if content.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = content
        .split('\n')
        .map(|line| line.trim_end_matches('\r').to_string())
        .collect();
    if content.ends_with('\n') {
        lines.pop();
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(path, content)| (path.to_string(), content.to_string()))
            .collect()
    }

    fn update(path: &str, hunk: Hunk) -> Patch {
        Patch {
            operations: vec![FileOperation::Update {
                path: path.to_string(),
                move_to: None,
                hunks: vec![hunk],
            }],
        }
    }

    #[test]
    fn replace_once_requires_exactly_one_occurrence() {
        assert_eq!(replace_once("a b a", "b", "c").as_deref(), Some("a c a"));
        assert!(replace_once("a b a", "a", "c").is_none(), "two occurrences");
        assert!(replace_once("abc", "z", "c").is_none(), "no occurrence");
        assert!(replace_once("abc", "", "c").is_none(), "empty needle");
    }

    #[test]
    fn replace_once_preserves_multibyte_content_around_the_edit() {
        let before = "前置\nold\n後置\n";
        let after = replace_once(before, "old", "新しい").unwrap();
        assert_eq!(after, "前置\n新しい\n後置\n");
    }

    #[test]
    fn a_crlf_file_stays_crlf_after_an_edit() {
        let before = "one\r\ntwo\r\nthree\r\n";
        let patch = update(
            "a.txt",
            Hunk {
                removed: vec!["two".to_string()],
                added: vec!["TWO".to_string()],
                ..Hunk::default()
            },
        );
        let outcome = apply_patch(&files(&[("a.txt", before)]), &patch).unwrap();
        let after = outcome.changes[0].after.as_deref().unwrap();
        assert_eq!(after, "one\r\nTWO\r\nthree\r\n");
        assert!(!after.contains("\n\n"));
    }

    #[test]
    fn an_lf_file_stays_lf_and_a_mixed_file_normalizes_to_lf() {
        let patch = update(
            "a.txt",
            Hunk {
                removed: vec!["two".to_string()],
                added: vec!["TWO".to_string()],
                ..Hunk::default()
            },
        );
        let lf = apply_patch(&files(&[("a.txt", "one\ntwo\n")]), &patch).unwrap();
        assert_eq!(lf.changes[0].after.as_deref().unwrap(), "one\nTWO\n");

        let mixed = apply_patch(&files(&[("a.txt", "one\r\ntwo\n")]), &patch).unwrap();
        assert_eq!(mixed.changes[0].after.as_deref().unwrap(), "one\nTWO\n");
    }

    #[test]
    fn a_file_without_a_trailing_newline_does_not_gain_one() {
        let patch = update(
            "a.txt",
            Hunk {
                removed: vec!["two".to_string()],
                added: vec!["TWO".to_string()],
                ..Hunk::default()
            },
        );
        let outcome = apply_patch(&files(&[("a.txt", "one\ntwo")]), &patch).unwrap();
        assert_eq!(outcome.changes[0].after.as_deref().unwrap(), "one\nTWO");
    }

    #[test]
    fn a_reindented_hunk_applies_and_reports_a_fuzzy_warning() {
        let patch = update(
            "a.rs",
            Hunk {
                context_before: vec!["fn main() {".to_string()],
                removed: vec!["    let x = 1;".to_string()],
                added: vec!["    let x = 2;".to_string()],
                ..Hunk::default()
            },
        );
        let outcome = apply_patch(
            &files(&[("a.rs", "fn main() {\n\t\tlet x = 1;\n}\n")]),
            &patch,
        )
        .unwrap();
        assert_eq!(
            outcome.changes[0].after.as_deref().unwrap(),
            "fn main() {\n    let x = 2;\n}\n"
        );
        assert!(outcome.had_fuzzy_matches());
        assert!(outcome.warnings[0].contains("whitespace-insensitive"));
    }

    #[test]
    fn an_exact_match_produces_no_warnings() {
        let patch = update(
            "a.txt",
            Hunk {
                removed: vec!["two".to_string()],
                added: vec!["TWO".to_string()],
                ..Hunk::default()
            },
        );
        let outcome = apply_patch(&files(&[("a.txt", "one\ntwo\n")]), &patch).unwrap();
        assert!(!outcome.had_fuzzy_matches());
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn missing_context_is_retryable_but_a_missing_input_is_not() {
        let patch = update(
            "a.txt",
            Hunk {
                removed: vec!["nope".to_string()],
                added: vec!["x".to_string()],
                ..Hunk::default()
            },
        );
        let error = apply_patch(&files(&[("a.txt", "one\n")]), &patch).unwrap_err();
        assert!(matches!(error, ApplyError::ContextNotFound { .. }));
        assert!(error.is_retryable(), "the model can re-read and retry");

        let absent = apply_patch(&BTreeMap::new(), &patch).unwrap_err();
        assert!(matches!(absent, ApplyError::MissingInput { .. }));
        assert!(!absent.is_retryable(), "retrying cannot conjure the file");
    }

    #[test]
    fn ambiguous_context_is_rejected_and_retryable() {
        let patch = update(
            "a.txt",
            Hunk {
                removed: vec!["dup".to_string()],
                added: vec!["x".to_string()],
                ..Hunk::default()
            },
        );
        let error = apply_patch(&files(&[("a.txt", "dup\nmid\ndup\n")]), &patch).unwrap_err();
        match &error {
            ApplyError::AmbiguousContext { occurrences, .. } => assert_eq!(*occurrences, 2),
            other => panic!("expected ambiguity, got {other:?}"),
        }
        assert!(error.is_retryable());
    }

    #[test]
    fn a_heading_disambiguates_otherwise_ambiguous_context() {
        let patch = update(
            "a.rs",
            Hunk {
                heading: Some("fn second()".to_string()),
                removed: vec!["    body".to_string()],
                added: vec!["    changed".to_string()],
                ..Hunk::default()
            },
        );
        let source = "fn first()\n    body\nfn second()\n    body\n";
        let outcome = apply_patch(&files(&[("a.rs", source)]), &patch).unwrap();
        assert_eq!(
            outcome.changes[0].after.as_deref().unwrap(),
            "fn first()\n    body\nfn second()\n    changed\n",
            "the heading selects the second occurrence"
        );
    }

    #[test]
    fn adding_over_an_existing_file_is_a_hard_failure() {
        let patch = Patch {
            operations: vec![FileOperation::Add {
                path: "a.txt".to_string(),
                content: "new\n".to_string(),
            }],
        };
        let error = apply_patch(&files(&[("a.txt", "old\n")]), &patch).unwrap_err();
        assert!(matches!(error, ApplyError::AlreadyExists { .. }));
        assert!(!error.is_retryable());
    }

    #[test]
    fn touching_the_same_path_twice_is_rejected() {
        let patch = Patch {
            operations: vec![
                FileOperation::Delete {
                    path: "a.txt".to_string(),
                },
                FileOperation::Delete {
                    path: "a.txt".to_string(),
                },
            ],
        };
        let error = apply_patch(&files(&[("a.txt", "x\n")]), &patch).unwrap_err();
        assert!(matches!(error, ApplyError::DuplicatePath { .. }));
        assert!(!error.is_retryable());
    }

    #[test]
    fn two_hunks_in_one_file_apply_in_order() {
        let patch = Patch {
            operations: vec![FileOperation::Update {
                path: "a.txt".to_string(),
                move_to: None,
                hunks: vec![
                    Hunk {
                        removed: vec!["one".to_string()],
                        added: vec!["ONE".to_string()],
                        ..Hunk::default()
                    },
                    Hunk {
                        removed: vec!["three".to_string()],
                        added: vec!["THREE".to_string()],
                        ..Hunk::default()
                    },
                ],
            }],
        };
        let outcome = apply_patch(&files(&[("a.txt", "one\ntwo\nthree\n")]), &patch).unwrap();
        assert_eq!(
            outcome.changes[0].after.as_deref().unwrap(),
            "ONE\ntwo\nTHREE\n"
        );
    }

    #[test]
    fn a_pure_insertion_anchors_on_context_without_removing_anything() {
        let patch = update(
            "a.txt",
            Hunk {
                context_before: vec!["one".to_string()],
                added: vec!["inserted".to_string()],
                ..Hunk::default()
            },
        );
        let outcome = apply_patch(&files(&[("a.txt", "one\ntwo\n")]), &patch).unwrap();
        assert_eq!(
            outcome.changes[0].after.as_deref().unwrap(),
            "one\ninserted\ntwo\n"
        );
    }

    #[test]
    fn a_pure_deletion_drops_the_line() {
        let patch = update(
            "a.txt",
            Hunk {
                removed: vec!["two".to_string()],
                ..Hunk::default()
            },
        );
        let outcome = apply_patch(&files(&[("a.txt", "one\ntwo\nthree\n")]), &patch).unwrap();
        assert_eq!(outcome.changes[0].after.as_deref().unwrap(), "one\nthree\n");
    }

    #[test]
    fn a_failure_on_a_later_file_yields_no_partial_changes() {
        let patch = Patch {
            operations: vec![
                FileOperation::Update {
                    path: "good.txt".to_string(),
                    move_to: None,
                    hunks: vec![Hunk {
                        removed: vec!["a".to_string()],
                        added: vec!["A".to_string()],
                        ..Hunk::default()
                    }],
                },
                FileOperation::Update {
                    path: "bad.txt".to_string(),
                    move_to: None,
                    hunks: vec![Hunk {
                        removed: vec!["missing".to_string()],
                        added: vec!["x".to_string()],
                        ..Hunk::default()
                    }],
                },
            ],
        };
        let error =
            apply_patch(&files(&[("good.txt", "a\n"), ("bad.txt", "b\n")]), &patch).unwrap_err();
        assert_eq!(error.path(), "bad.txt");
        // The caller receives Err, so nothing is written for good.txt either.
    }

    #[test]
    fn a_move_is_reported_on_the_change() {
        let patch = Patch {
            operations: vec![FileOperation::Update {
                path: "old.txt".to_string(),
                move_to: Some("new.txt".to_string()),
                hunks: vec![Hunk {
                    removed: vec!["x".to_string()],
                    added: vec!["y".to_string()],
                    ..Hunk::default()
                }],
            }],
        };
        let outcome = apply_patch(&files(&[("old.txt", "x\n")]), &patch).unwrap();
        assert_eq!(outcome.changes[0].move_to.as_deref(), Some("new.txt"));
        assert_eq!(outcome.changes[0].kind, FileChangeKind::Update);
    }

    #[test]
    fn unicode_lines_survive_a_hunk_that_edits_neighbors() {
        let source = "絵文字 🎉\ntarget\n日本語\n";
        let patch = update(
            "a.txt",
            Hunk {
                removed: vec!["target".to_string()],
                added: vec!["置換".to_string()],
                ..Hunk::default()
            },
        );
        let outcome = apply_patch(&files(&[("a.txt", source)]), &patch).unwrap();
        assert_eq!(
            outcome.changes[0].after.as_deref().unwrap(),
            "絵文字 🎉\n置換\n日本語\n"
        );
    }

    #[test]
    fn overlapping_hunks_are_rejected() {
        // Both hunks target the same single line via identical context.
        let patch = Patch {
            operations: vec![FileOperation::Update {
                path: "a.txt".to_string(),
                move_to: None,
                hunks: vec![
                    Hunk {
                        removed: vec!["mid".to_string()],
                        added: vec!["first".to_string()],
                        ..Hunk::default()
                    },
                    Hunk {
                        context_before: vec!["first".to_string()],
                        added: vec!["second".to_string()],
                        ..Hunk::default()
                    },
                ],
            }],
        };
        // The second hunk anchors on the line the first just wrote, which is a
        // claimed region.
        let error = apply_patch(&files(&[("a.txt", "top\nmid\nend\n")]), &patch).unwrap_err();
        assert!(
            matches!(error, ApplyError::OverlappingHunks { .. }),
            "got {error:?}"
        );
        assert!(!error.is_retryable());
    }

    #[test]
    fn an_empty_file_can_receive_an_insertion() {
        let patch = update(
            "a.txt",
            Hunk {
                added: vec!["first".to_string()],
                ..Hunk::default()
            },
        );
        let outcome = apply_patch(&files(&[("a.txt", "")]), &patch).unwrap();
        assert_eq!(outcome.changes[0].after.as_deref().unwrap(), "first");
    }
}
