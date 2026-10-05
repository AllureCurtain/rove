//! The secret authority is a no-op in a process that never declared anything.
//!
//! Every other redaction contract in the repository is asserted in a binary where
//! some test has armed the process-wide registry, or against a detached
//! `SecretRegistry` that proves the pass's *logic* but not the behavior a writing
//! boundary actually has. This file is its own test binary precisely because the
//! registry is a process global: it must contain no registration at all, so that
//! the trace writer below runs with nothing declared.
//!
//! Do not register a value or a field name here. A single registration arms the
//! registry for this whole binary (arming is global and sticky), and the
//! precondition assertions would then fail rather than the contract being
//! silently untested — which is the point of keeping them.

use rove_runtime::events::{StreamEvent, TraceEntry};
use rove_runtime::state::trace::{TraceLine, TraceWriter};
use rove_runtime::types::CallId;

/// The bytes a real writing boundary produces are unchanged with nothing declared.
#[test]
fn an_unarmed_authority_leaves_written_bytes_untouched() {
    let rendered = format!("{:?}", rove_runtime::secrets::registry());
    assert!(
        rendered.contains("armed: false"),
        "this test binary must not have declared a credential: {rendered}"
    );
    assert!(rendered.contains("known_values: 0"), "{rendered}");
    assert!(rendered.contains("declared_fields: 0"), "{rendered}");

    // A payload with the two shapes the authority's passes look for: a value the
    // pattern backstop cannot recognise as a credential by shape
    // (`unregistered-canary-9f31`), and one written as a bare JSON number
    // (`9876543210`), which the pattern backstop never matches at all.
    let args = serde_json::json!({
        "token": 9876543210u64,
        "note": "unregistered-canary-9f31 rides through",
    });
    let event = StreamEvent::ToolCallStarted {
        call_id: CallId::new(),
        tool_use_id: None,
        name: "shell".to_string(),
        args: args.clone(),
    };

    let temp = tempfile::TempDir::new().unwrap();
    let writer = TraceWriter::new(temp.path()).unwrap();
    writer
        .append(&event)
        .expect("the trace writer accepts the event");

    let raw = std::fs::read_to_string(writer.path()).unwrap();
    let line = raw.trim_end();
    // The evidence is on the disk, in the bytes that were written.
    assert!(line.contains("unregistered-canary-9f31"), "{line}");
    assert!(line.contains("9876543210"), "{line}");
    assert!(!line.contains("[REDACTED"), "{line}");
    assert!(!line.contains("[TRUNCATED"), "{line}");

    // And the event's bytes inside the line are exactly the bytes `serde_json`
    // produces for the event that was appended: nothing re-serialized it, dropped
    // a field, or normalized the number.
    let decoded: TraceLine = serde_json::from_str(line).expect("the line is a trace line");
    assert_eq!(
        serde_json::to_string(&decoded.event).unwrap(),
        serde_json::to_string(&TraceEntry::Ui(event)).unwrap(),
        "the stored event is byte-identical to the appended event"
    );

    // The write path did not arm anything either: the authority is still inert
    // for whatever this process does next.
    let rendered = format!("{:?}", rove_runtime::secrets::registry());
    assert!(rendered.contains("armed: false"), "{rendered}");
}
