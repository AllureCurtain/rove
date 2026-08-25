//! Patch model and heredoc parser.
//!
//! The wire format mirrors codex's `apply_patch` heredoc so model output can be
//! shared between the two ecosystems:
//!
//! ```text
//! *** Begin Patch
//! *** Add File: docs/new.md
//! +first line
//! +second line
//! *** Update File: src/main.rs
//! @@ fn main() {
//!  let x = 1;
//! -    println!("old");
//! +    println!("new");
//! *** Delete File: obsolete.txt
//! *** End Patch
//! ```
//!
//! Parsing is pure and total: every failure is a typed [`PatchParseError`] that
//! names the offending line, so a model can correct its own output.

use serde::{Deserialize, Serialize};

const BEGIN: &str = "*** Begin Patch";
const END: &str = "*** End Patch";
const ADD: &str = "*** Add File: ";
const DELETE: &str = "*** Delete File: ";
const UPDATE: &str = "*** Update File: ";
const MOVE: &str = "*** Move to: ";
const HUNK: &str = "@@";

/// A parsed patch: an ordered list of per-file operations.
///
/// Order is preserved because operations on the same path must apply in the
/// order written, and callers may surface progress per file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Patch {
    pub operations: Vec<FileOperation>,
}

/// One file's requested change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileOperation {
    /// Create a new file with exactly `content`.
    Add { path: String, content: String },
    /// Remove an existing file.
    Delete { path: String },
    /// Apply `hunks` in order to an existing file, optionally renaming it.
    Update {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        move_to: Option<String>,
        hunks: Vec<Hunk>,
    },
}

impl FileOperation {
    /// The path this operation reads from.
    pub fn path(&self) -> &str {
        match self {
            Self::Add { path, .. } | Self::Delete { path } | Self::Update { path, .. } => path,
        }
    }
}

/// One contiguous edit within a file.
///
/// `context_before` and `context_after` are unchanged anchor lines,
/// `removed` are lines the patch expects to find and drop, and `added` are the
/// lines to insert in their place. A hunk with empty `removed` is an insertion;
/// empty `added` is a deletion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Hunk {
    /// Optional `@@ <text>` section heading used as a coarse locator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading: Option<String>,
    #[serde(default)]
    pub context_before: Vec<String>,
    #[serde(default)]
    pub removed: Vec<String>,
    #[serde(default)]
    pub added: Vec<String>,
    #[serde(default)]
    pub context_after: Vec<String>,
}

impl Hunk {
    /// The lines this hunk expects to find in the file, in order: leading
    /// context, then the lines it will remove, then trailing context.
    pub fn expected_lines(&self) -> Vec<&str> {
        let mut lines = Vec::with_capacity(
            self.context_before.len() + self.removed.len() + self.context_after.len(),
        );
        lines.extend(self.context_before.iter().map(String::as_str));
        lines.extend(self.removed.iter().map(String::as_str));
        lines.extend(self.context_after.iter().map(String::as_str));
        lines
    }

    /// The lines that replace [`Self::expected_lines`] after a successful apply.
    pub fn replacement_lines(&self) -> Vec<&str> {
        let mut lines = Vec::with_capacity(
            self.context_before.len() + self.added.len() + self.context_after.len(),
        );
        lines.extend(self.context_before.iter().map(String::as_str));
        lines.extend(self.added.iter().map(String::as_str));
        lines.extend(self.context_after.iter().map(String::as_str));
        lines
    }

    /// True when the hunk neither removes nor adds anything, which would make
    /// applying it a silent no-op.
    pub fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.added.is_empty()
    }
}

/// Why a patch could not be parsed. Every variant names a recoverable mistake
/// so the model can re-emit a corrected patch.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PatchParseError {
    #[error("patch must start with `{BEGIN}`")]
    MissingBegin,
    #[error("patch must end with `{END}`")]
    MissingEnd,
    #[error("line {line}: content appears before any `*** Add/Update/Delete File:` header")]
    ContentBeforeHeader { line: usize },
    #[error("line {line}: `{directive}` requires a non-empty path")]
    EmptyPath { line: usize, directive: String },
    #[error("line {line}: unrecognized line `{content}`")]
    UnrecognizedLine { line: usize, content: String },
    #[error("line {line}: `{MOVE}` is only valid inside an `{UPDATE}` section")]
    MisplacedMove { line: usize },
    #[error("line {line}: hunk in `{path}` changes nothing")]
    EmptyHunk { line: usize, path: String },
    #[error("`{UPDATE}{path}` declares no hunks")]
    UpdateWithoutHunks { path: String },
    #[error("patch declares no file operations")]
    Empty,
}

/// Parse a heredoc patch into a [`Patch`].
///
/// Accepts both LF and CRLF line endings; the parser strips the carriage return
/// so patches authored on Windows behave identically. Content lines keep their
/// own trailing whitespace.
pub fn parse_patch(input: &str) -> Result<Patch, PatchParseError> {
    let raw_lines: Vec<&str> = input
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .collect();
    let mut cursor = 0;
    while cursor < raw_lines.len() && raw_lines[cursor].trim().is_empty() {
        cursor += 1;
    }
    if cursor >= raw_lines.len() || raw_lines[cursor].trim() != BEGIN {
        return Err(PatchParseError::MissingBegin);
    }
    cursor += 1;

    let mut parser = Parser::default();
    let mut saw_end = false;
    while cursor < raw_lines.len() {
        let line_number = cursor + 1;
        let line = raw_lines[cursor];
        cursor += 1;

        if line.trim() == END {
            saw_end = true;
            break;
        }
        parser.consume(line, line_number)?;
    }
    if !saw_end {
        return Err(PatchParseError::MissingEnd);
    }
    parser.finish()
}

/// Incremental parse state for one patch.
#[derive(Default)]
struct Parser {
    operations: Vec<FileOperation>,
    section: Option<Section>,
}

enum Section {
    Add {
        path: String,
        lines: Vec<String>,
    },
    Update {
        path: String,
        move_to: Option<String>,
        hunks: Vec<Hunk>,
        current: Option<Hunk>,
        /// Set once a hunk has begun emitting removals/additions, so trailing
        /// context is distinguished from leading context.
        past_changes: bool,
    },
}

impl Parser {
    fn consume(&mut self, line: &str, line_number: usize) -> Result<(), PatchParseError> {
        if let Some(rest) = line.strip_prefix(ADD) {
            self.flush()?;
            let path = require_path(rest, line_number, ADD)?;
            self.section = Some(Section::Add {
                path,
                lines: Vec::new(),
            });
            return Ok(());
        }
        if let Some(rest) = line.strip_prefix(DELETE) {
            self.flush()?;
            let path = require_path(rest, line_number, DELETE)?;
            self.operations.push(FileOperation::Delete { path });
            return Ok(());
        }
        if let Some(rest) = line.strip_prefix(UPDATE) {
            self.flush()?;
            let path = require_path(rest, line_number, UPDATE)?;
            self.section = Some(Section::Update {
                path,
                move_to: None,
                hunks: Vec::new(),
                current: None,
                past_changes: false,
            });
            return Ok(());
        }
        if let Some(rest) = line.strip_prefix(MOVE) {
            let target = require_path(rest, line_number, MOVE)?;
            return match self.section.as_mut() {
                Some(Section::Update { move_to, .. }) => {
                    *move_to = Some(target);
                    Ok(())
                }
                _ => Err(PatchParseError::MisplacedMove { line: line_number }),
            };
        }
        self.consume_body(line, line_number)
    }

    fn consume_body(&mut self, line: &str, line_number: usize) -> Result<(), PatchParseError> {
        match self.section.as_mut() {
            None => {
                if line.trim().is_empty() {
                    return Ok(());
                }
                Err(PatchParseError::ContentBeforeHeader { line: line_number })
            }
            Some(Section::Add { lines, .. }) => {
                // Added files carry `+` prefixes; a bare blank line is a blank
                // line in the new file.
                if let Some(rest) = line.strip_prefix('+') {
                    lines.push(rest.to_string());
                    Ok(())
                } else if line.trim().is_empty() {
                    lines.push(String::new());
                    Ok(())
                } else {
                    Err(PatchParseError::UnrecognizedLine {
                        line: line_number,
                        content: line.to_string(),
                    })
                }
            }
            Some(Section::Update {
                path,
                hunks,
                current,
                past_changes,
                ..
            }) => Self::consume_update_body(line, line_number, path, hunks, current, past_changes),
        }
    }

    fn consume_update_body(
        line: &str,
        line_number: usize,
        path: &str,
        hunks: &mut Vec<Hunk>,
        current: &mut Option<Hunk>,
        past_changes: &mut bool,
    ) -> Result<(), PatchParseError> {
        if let Some(rest) = line.strip_prefix(HUNK) {
            if let Some(finished) = current.take() {
                if finished.is_empty() {
                    return Err(PatchParseError::EmptyHunk {
                        line: line_number,
                        path: path.to_string(),
                    });
                }
                hunks.push(finished);
            }
            let heading = rest.trim();
            *current = Some(Hunk {
                heading: (!heading.is_empty()).then(|| heading.to_string()),
                ..Hunk::default()
            });
            *past_changes = false;
            return Ok(());
        }

        let hunk = current.get_or_insert_with(Hunk::default);
        match line.chars().next() {
            Some('+') => {
                hunk.added.push(line[1..].to_string());
                *past_changes = true;
                Ok(())
            }
            Some('-') => {
                hunk.removed.push(line[1..].to_string());
                *past_changes = true;
                Ok(())
            }
            Some(' ') => {
                let content = line[1..].to_string();
                if *past_changes {
                    hunk.context_after.push(content);
                } else {
                    hunk.context_before.push(content);
                }
                Ok(())
            }
            // A truly empty line inside an update section is context for a
            // blank source line; models frequently omit its leading space.
            None => {
                if *past_changes {
                    hunk.context_after.push(String::new());
                } else {
                    hunk.context_before.push(String::new());
                }
                Ok(())
            }
            Some(_) => Err(PatchParseError::UnrecognizedLine {
                line: line_number,
                content: line.to_string(),
            }),
        }
    }

    fn flush(&mut self) -> Result<(), PatchParseError> {
        match self.section.take() {
            None => Ok(()),
            Some(Section::Add { path, lines }) => {
                let mut content = lines.join("\n");
                if !content.is_empty() {
                    content.push('\n');
                }
                self.operations.push(FileOperation::Add { path, content });
                Ok(())
            }
            Some(Section::Update {
                path,
                move_to,
                mut hunks,
                current,
                ..
            }) => {
                if let Some(last) = current {
                    if last.is_empty() {
                        return Err(PatchParseError::EmptyHunk { line: 0, path });
                    }
                    hunks.push(last);
                }
                if hunks.is_empty() {
                    return Err(PatchParseError::UpdateWithoutHunks { path });
                }
                self.operations.push(FileOperation::Update {
                    path,
                    move_to,
                    hunks,
                });
                Ok(())
            }
        }
    }

    fn finish(mut self) -> Result<Patch, PatchParseError> {
        self.flush()?;
        if self.operations.is_empty() {
            return Err(PatchParseError::Empty);
        }
        Ok(Patch {
            operations: self.operations,
        })
    }
}

fn require_path(raw: &str, line: usize, directive: &str) -> Result<String, PatchParseError> {
    let path = raw.trim();
    if path.is_empty() {
        return Err(PatchParseError::EmptyPath {
            line,
            directive: directive.trim().to_string(),
        });
    }
    Ok(path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_patch_parses_all_three_operation_kinds() {
        let input = "\
*** Begin Patch
*** Add File: docs/new.md
+# Title
+body
*** Update File: src/main.rs
@@ fn main() {
 let x = 1;
-    println!(\"old\");
+    println!(\"new\");
*** Delete File: obsolete.txt
*** End Patch
";
        let patch = parse_patch(input).unwrap();
        assert_eq!(patch.operations.len(), 3);

        match &patch.operations[0] {
            FileOperation::Add { path, content } => {
                assert_eq!(path, "docs/new.md");
                assert_eq!(content, "# Title\nbody\n");
            }
            other => panic!("expected Add, got {other:?}"),
        }
        match &patch.operations[1] {
            FileOperation::Update { path, hunks, .. } => {
                assert_eq!(path, "src/main.rs");
                assert_eq!(hunks.len(), 1);
                assert_eq!(hunks[0].heading.as_deref(), Some("fn main() {"));
                assert_eq!(hunks[0].context_before, vec!["let x = 1;"]);
                assert_eq!(hunks[0].removed, vec!["    println!(\"old\");"]);
                assert_eq!(hunks[0].added, vec!["    println!(\"new\");"]);
            }
            other => panic!("expected Update, got {other:?}"),
        }
        assert_eq!(patch.operations[2].path(), "obsolete.txt");
    }

    #[test]
    fn crlf_authored_patches_parse_identically_to_lf() {
        let lf = "*** Begin Patch\n*** Delete File: a.txt\n*** End Patch\n";
        let crlf = "*** Begin Patch\r\n*** Delete File: a.txt\r\n*** End Patch\r\n";
        assert_eq!(parse_patch(lf).unwrap(), parse_patch(crlf).unwrap());
    }

    #[test]
    fn context_after_a_change_is_separated_from_context_before() {
        let input = "\
*** Begin Patch
*** Update File: a.txt
@@
 before
-old
+new
 after
*** End Patch
";
        let patch = parse_patch(input).unwrap();
        match &patch.operations[0] {
            FileOperation::Update { hunks, .. } => {
                assert_eq!(hunks[0].context_before, vec!["before"]);
                assert_eq!(hunks[0].context_after, vec!["after"]);
                assert_eq!(hunks[0].heading, None, "a bare @@ carries no heading");
            }
            other => panic!("expected Update, got {other:?}"),
        }
    }

    #[test]
    fn multiple_hunks_in_one_file_are_split_on_the_marker() {
        let input = "\
*** Begin Patch
*** Update File: a.txt
@@
-one
+ONE
@@
-two
+TWO
*** End Patch
";
        match &parse_patch(input).unwrap().operations[0] {
            FileOperation::Update { hunks, .. } => assert_eq!(hunks.len(), 2),
            other => panic!("expected Update, got {other:?}"),
        }
    }

    #[test]
    fn a_move_directive_attaches_to_the_update() {
        let input = "\
*** Begin Patch
*** Update File: old.txt
*** Move to: new.txt
@@
-x
+y
*** End Patch
";
        match &parse_patch(input).unwrap().operations[0] {
            FileOperation::Update { move_to, .. } => {
                assert_eq!(move_to.as_deref(), Some("new.txt"));
            }
            other => panic!("expected Update, got {other:?}"),
        }
    }

    #[test]
    fn missing_sentinels_are_named_precisely() {
        assert_eq!(
            parse_patch("*** Delete File: a.txt\n").unwrap_err(),
            PatchParseError::MissingBegin
        );
        assert_eq!(
            parse_patch("*** Begin Patch\n*** Delete File: a.txt\n").unwrap_err(),
            PatchParseError::MissingEnd
        );
    }

    #[test]
    fn structural_mistakes_report_their_line() {
        let stray = "*** Begin Patch\nstray text\n*** End Patch\n";
        assert_eq!(
            parse_patch(stray).unwrap_err(),
            PatchParseError::ContentBeforeHeader { line: 2 }
        );

        let misplaced = "*** Begin Patch\n*** Move to: b.txt\n*** End Patch\n";
        assert_eq!(
            parse_patch(misplaced).unwrap_err(),
            PatchParseError::MisplacedMove { line: 2 }
        );

        let empty_path = "*** Begin Patch\n*** Add File: \n*** End Patch\n";
        match parse_patch(empty_path).unwrap_err() {
            PatchParseError::EmptyPath { line, .. } => assert_eq!(line, 2),
            other => panic!("expected EmptyPath, got {other:?}"),
        }
    }

    #[test]
    fn an_update_without_hunks_is_rejected() {
        let input = "*** Begin Patch\n*** Update File: a.txt\n*** End Patch\n";
        assert_eq!(
            parse_patch(input).unwrap_err(),
            PatchParseError::UpdateWithoutHunks {
                path: "a.txt".to_string()
            }
        );
    }

    #[test]
    fn an_empty_patch_is_rejected() {
        assert_eq!(
            parse_patch("*** Begin Patch\n*** End Patch\n").unwrap_err(),
            PatchParseError::Empty
        );
    }

    #[test]
    fn an_unrecognized_body_line_names_its_content() {
        let input = "\
*** Begin Patch
*** Update File: a.txt
@@
?bogus
*** End Patch
";
        match parse_patch(input).unwrap_err() {
            PatchParseError::UnrecognizedLine { line, content } => {
                assert_eq!(line, 4);
                assert_eq!(content, "?bogus");
            }
            other => panic!("expected UnrecognizedLine, got {other:?}"),
        }
    }

    #[test]
    fn an_added_file_preserves_blank_lines_and_unicode() {
        let input = "\
*** Begin Patch
*** Add File: a.md
+# 標題
+
+本文 🎉
*** End Patch
";
        match &parse_patch(input).unwrap().operations[0] {
            FileOperation::Add { content, .. } => {
                assert_eq!(content, "# 標題\n\n本文 🎉\n");
            }
            other => panic!("expected Add, got {other:?}"),
        }
    }

    #[test]
    fn hunk_line_projections_round_trip_expected_and_replacement() {
        let hunk = Hunk {
            heading: None,
            context_before: vec!["a".to_string()],
            removed: vec!["b".to_string()],
            added: vec!["B".to_string()],
            context_after: vec!["c".to_string()],
        };
        assert_eq!(hunk.expected_lines(), vec!["a", "b", "c"]);
        assert_eq!(hunk.replacement_lines(), vec!["a", "B", "c"]);
        assert!(!hunk.is_empty());
        assert!(Hunk::default().is_empty());
    }

    #[test]
    fn a_patch_round_trips_through_serde() {
        let input = "\
*** Begin Patch
*** Update File: a.txt
@@ section
-old
+new
*** End Patch
";
        let patch = parse_patch(input).unwrap();
        let json = serde_json::to_string(&patch).unwrap();
        assert_eq!(serde_json::from_str::<Patch>(&json).unwrap(), patch);
    }
}
