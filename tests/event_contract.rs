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

/// Marker of the runtime's canonical-event registry.
const KINDS_MARKER: &str = "STREAM_EVENT_KINDS: &[(&str, u32)] = &[";
/// Marker of the runtime's current contract version.
const RUST_CONTRACT_VERSION_MARKER: &str = "STREAM_EVENT_CONTRACT_VERSION: u32 = ";
/// Marker of the Web bundle's declared contract version.
const WEB_CONTRACT_VERSION_MARKER: &str = "STREAM_EVENT_CONTRACT_VERSION = ";

fn read_events_rs() -> String {
    std::fs::read_to_string(workspace_path(EVENTS_RS))
        .unwrap_or_else(|err| panic!("failed to read {EVENTS_RS}: {err}"))
}

/// Event names returned by `StreamEvent::event_name` in source order.
fn rust_event_names() -> Vec<String> {
    let source = read_events_rs();
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

/// `(name, contract version)` entries of the runtime registry, in source order.
fn rust_stream_event_kinds() -> Vec<(String, u32)> {
    let source = read_events_rs();
    let start = source
        .find(KINDS_MARKER)
        .unwrap_or_else(|| panic!("{EVENTS_RS} should declare {KINDS_MARKER}"));
    let block = &source[start..];
    let end = block
        .find("\n];")
        .expect("the canonical-event registry should close its array");
    block[..end]
        .lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("(\"")?;
            let name_end = rest.find('"')?;
            let version: u32 = rest[name_end..]
                .trim_matches(|character: char| !character.is_ascii_digit())
                .parse()
                .ok()?;
            Some((rest[..name_end].to_string(), version))
        })
        .collect()
}

/// The runtime's current canonical-event contract version.
fn rust_contract_version() -> u32 {
    let source = read_events_rs();
    let start = source
        .find(RUST_CONTRACT_VERSION_MARKER)
        .unwrap_or_else(|| panic!("{EVENTS_RS} should declare {RUST_CONTRACT_VERSION_MARKER}"));
    let rest = &source[start + RUST_CONTRACT_VERSION_MARKER.len()..];
    let end = rest.find(';').expect("the const should end with ';'");
    rest[..end]
        .trim()
        .parse()
        .expect("the contract version should be an integer literal")
}

/// The canonical-event contract version the Web bundle declares.
fn web_declared_contract_version() -> u32 {
    let source = read_web_types();
    let start = source
        .find(WEB_CONTRACT_VERSION_MARKER)
        .unwrap_or_else(|| panic!("{WEB_TYPES_TS} should declare {WEB_CONTRACT_VERSION_MARKER}"));
    let rest = &source[start + WEB_CONTRACT_VERSION_MARKER.len()..];
    let end = rest.find(';').expect("the const should end with ';'");
    rest[..end]
        .trim()
        .parse()
        .expect("the declared contract version should be a numeric literal")
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

#[test]
fn canonical_event_registry_covers_the_wire_names_with_bounded_versions() {
    let names = rust_event_names();
    let kinds = rust_stream_event_kinds();
    let kind_names: Vec<String> = kinds.iter().map(|(name, _)| name.clone()).collect();

    assert!(
        kinds.len() >= 16,
        "expected to parse the full canonical-event registry, found {}: {kinds:?}",
        kinds.len()
    );
    assert_eq!(
        kind_names,
        names,
        "the canonical-event registry and StreamEvent::event_name drifted.\n  \
         only in event_name: {:?}\n  only in registry: {:?}\n\
         Update {KINDS_MARKER} in {EVENTS_RS}.",
        difference(&names, &kind_names),
        difference(&kind_names, &names),
    );

    let current = rust_contract_version();
    let versions: Vec<u32> = kinds.iter().map(|(_, version)| *version).collect();
    assert!(
        versions
            .iter()
            .all(|version| *version >= 1 && *version <= current),
        "every contract version must be between 1 and {current}: {kinds:?}"
    );
    assert_eq!(
        versions.iter().max().copied(),
        Some(current),
        "STREAM_EVENT_CONTRACT_VERSION must be the newest version in the registry"
    );

    // The transcript `event_contract` negotiation is only meaningful while a kind
    // newer than the oldest contract exists; `provider_retry` is that kind, so R2c
    // stays a real canonical event instead of a private side channel.
    let newer: Vec<&str> = kinds
        .iter()
        .filter(|(_, version)| *version > 1)
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(
        newer,
        vec!["provider_retry"],
        "provider_retry is the kind that introduced contract version 2"
    );
}

#[test]
fn web_bundle_declares_the_contract_of_the_kinds_it_lists() {
    let kinds = rust_stream_event_kinds();
    let web_names = web_const_names();
    let declared = web_declared_contract_version();

    let expected = kinds
        .iter()
        .filter(|(name, _)| web_names.contains(name))
        .map(|(_, version)| *version)
        .max()
        .expect("the Web bundle should name at least one canonical event");
    assert_eq!(
        declared, expected,
        "the Web bundle must declare the newest canonical-event contract version of the kinds it lists"
    );
    assert_eq!(
        declared,
        rust_contract_version(),
        "this Web bundle lists every current kind, so it must declare the current contract"
    );
}

/// The checked-in OpenAPI snapshot is the published contract for the SSE event
/// payloads: every canonical kind must surface as the `type` discriminator of
/// one `StreamEvent` variant, or generated clients cannot decode the stream.
/// Variants with `#[serde(flatten)]` fields carry the discriminator inside an
/// `allOf` member, so both shapes are searched. The snapshot file is the
/// artifact under test; `openapi_snapshot_matches_the_served_document` in
/// `tests/api.rs` pins it to the served document.
#[test]
fn openapi_snapshot_exposes_every_canonical_event_kind() {
    const OPENAPI_JSON: &str = "apps/api/openapi.json";

    fn discriminator(variant: &serde_json::Value) -> Option<String> {
        let inline = variant
            .pointer("/properties/type/enum/0")
            .and_then(serde_json::Value::as_str);
        inline.map(str::to_string).or_else(|| {
            variant.get("allOf")?.as_array()?.iter().find_map(|member| {
                member
                    .pointer("/properties/type/enum/0")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
        })
    }

    let document: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(workspace_path(OPENAPI_JSON))
            .unwrap_or_else(|err| panic!("failed to read {OPENAPI_JSON}: {err}")),
    )
    .expect("openapi.json should parse");
    let variants = document
        .pointer("/components/schemas/StreamEvent/oneOf")
        .and_then(serde_json::Value::as_array)
        .expect("StreamEvent should be an internally tagged oneOf schema");

    let kind_names: Vec<String> = rust_stream_event_kinds()
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    let names: Vec<String> = variants.iter().filter_map(discriminator).collect();

    assert_eq!(
        names.len(),
        variants.len(),
        "every StreamEvent variant must carry the `type` discriminator"
    );
    assert_eq!(
        names,
        kind_names,
        "the published StreamEvent schema and the canonical registry drifted.\n  \
         only in OpenAPI: {:?}\n  only in registry: {:?}\n  \
         Regenerate the snapshot with ROVE_UPDATE_OPENAPI=1.",
        difference(&names, &kind_names),
        difference(&kind_names, &names),
    );
}
