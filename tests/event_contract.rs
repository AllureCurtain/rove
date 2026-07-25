//! Cross-language contract guard for the streaming event surface.
//!
//! The runtime defines [`StreamEvent`](rove_runtime::events::StreamEvent) variants that
//! three consumers depend on: the CLI, the API/SSE layer, and the Web UI. The Rust
//! compiler already forces `StreamEvent::event_name` to cover every variant
//! (exhaustive match), so it is the authoritative list of wire event names. The Web
//! UI re-declares the same surface by hand in `apps/web/lib/rove-types.ts`
//! (`STREAM_EVENT_NAMES` plus the `StreamEvent` union discriminants).
//!
//! Those hand-written copies drift: a new Rust variant once shipped without the
//! matching Web type. These tests fail when the Rust and Web event surfaces diverge,
//! turning a silent contract drift into a red build.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.pop();
    root
}

fn workspace_path(rel: impl AsRef<Path>) -> PathBuf {
    workspace_root().join(rel)
}

const EVENTS_RS: &str = "runtime/src/foundation/events.rs";
const WEB_TYPES_TS: &str = "apps/web/lib/rove-types.ts";

/// Event names returned by `StreamEvent::event_name` in source order.
fn rust_event_names() -> Vec<String> {
    let source = std::fs::read_to_string(workspace_path(EVENTS_RS))
        .unwrap_or_else(|err| panic!("failed to read {EVENTS_RS}: {err}"));
    let fn_start = source
        .find("fn event_name")
        .expect("runtime/src/foundation/events.rs should define fn event_name");
    source[fn_start..]
        .lines()
        .filter_map(|line| {
            let arrow = line.find("=> \"")?;
            let rest = &line[arrow + 4..];
            let end = rest.find('"')?;
            Some(rest[..end].to_string())
        })
        .collect()
}

/// Names listed in the Web `STREAM_EVENT_NAMES` const array, in source order.
fn web_const_names() -> Vec<String> {
    let source = read_web_types();
    extract_ts_string_array(&source, "STREAM_EVENT_NAMES = [")
}

/// `type: "..."` discriminants of the Web `StreamEvent` union, in source order.
fn web_union_names() -> Vec<String> {
    let source = read_web_types();
    let start = source
        .find("export type StreamEvent =")
        .expect("rove-types.ts should declare export type StreamEvent");
    let block = &source[start..];
    let end = block.find("\n\nexport ").unwrap_or(block.len());
    block[..end]
        .lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("type: \"")?;
            let end = rest.find('"')?;
            Some(rest[..end].to_string())
        })
        .collect()
}

fn read_web_types() -> String {
    std::fs::read_to_string(workspace_path(WEB_TYPES_TS))
        .unwrap_or_else(|err| panic!("failed to read {WEB_TYPES_TS}: {err}"))
}

fn extract_ts_string_array(source: &str, marker: &str) -> Vec<String> {
    let start = source
        .find(marker)
        .unwrap_or_else(|| panic!("expected {marker:?} in {WEB_TYPES_TS}"));
    let after = &source[start..];
    let open = after.find('[').expect("array literal should have '['");
    let close = after[open..]
        .find(']')
        .expect("array literal should have ']'")
        + open;
    after[open + 1..close]
        .split(',')
        .map(|item| item.trim().trim_matches(|c| c == '"' || c == '\''))
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

fn difference(left: &[String], right: &[String]) -> Vec<String> {
    left.iter()
        .filter(|item| !right.contains(item))
        .cloned()
        .collect()
}

#[test]
fn rust_and_web_stream_event_names_match() {
    let rust = rust_event_names();
    let web = web_const_names();

    assert!(
        rust.len() >= 16,
        "expected to parse the full event_name match arms, found {}: {rust:?}",
        rust.len()
    );
    assert_eq!(
        rust,
        web,
        "Rust StreamEvent::event_name and Web STREAM_EVENT_NAMES drifted.\n  \
         only in Rust: {:?}\n  only in Web: {:?}\n\
         Update apps/web/lib/rove-types.ts to match runtime/src/foundation/events.rs.",
        difference(&rust, &web),
        difference(&web, &rust),
    );
}

#[test]
fn web_stream_event_union_matches_name_list() {
    let names = web_const_names();
    let union = web_union_names();

    assert_eq!(
        names,
        union,
        "Web STREAM_EVENT_NAMES and the StreamEvent union discriminants drifted.\n  \
         only in name list: {:?}\n  only in union: {:?}",
        difference(&names, &union),
        difference(&union, &names),
    );
}
