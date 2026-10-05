//! Text truncation shared by redacted text surfaces.

/// Truncate `value` to at most `max_bytes` on a character boundary, without
/// ending inside a redaction or truncation marker.
///
/// A cut inside `[REDACTED:known_secret]` leaves `[REDACTED:known_`, which reads
/// like a marker a reader is meant to understand and is not one. A marker that
/// does not fit within the budget is dropped whole instead. Returns the kept
/// prefix and whether anything was cut.
pub fn truncate_utf8_preserving_markers(value: &str, max_bytes: usize) -> (&str, bool) {
    if value.len() <= max_bytes {
        return (value, false);
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    (marker_safe_prefix(value, end), true)
}

/// The longest prefix of `value` that ends at or before `end` without splitting
/// a marker.
fn marker_safe_prefix(value: &str, end: usize) -> &str {
    /// Openings of the markers this repository writes into text surfaces.
    const MARKER_OPENINGS: [&str; 2] = ["[REDACTED:", "[TRUNCATED:"];
    let slice = &value[..end];
    let Some(open) = slice.rfind('[') else {
        return slice;
    };
    let tail = &slice[open..];
    if tail.contains(']') {
        // The bracket closes, so the cut is outside a marker.
        return slice;
    }
    // `tail` is either an opening that lost its `]` or a partial opening such as
    // `[RED`. An ordinary `[` that is not the start of a marker is left alone.
    let opens_marker = MARKER_OPENINGS
        .iter()
        .any(|opening| opening.starts_with(tail) || tail.starts_with(opening));
    if opens_marker { &slice[..open] } else { slice }
}
