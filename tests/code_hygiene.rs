#[test]
fn lib_rs_does_not_hide_dead_code_globally() {
    let lib = std::fs::read_to_string("src/lib.rs").unwrap();

    assert!(!lib.contains("#![allow(dead_code)]"));
}

#[test]
fn runtime_docs_record_phase_12_hygiene_and_source_of_truth_status() {
    let status = std::fs::read_to_string("docs/runtime/implementation-status.md").unwrap();
    let guide = std::fs::read_to_string("docs/runtime/implementation-guide.md").unwrap();
    let readme = std::fs::read_to_string("README.md").unwrap();

    assert!(status.contains("Dead code warnings are enforced"));
    assert!(status.contains("Runtime docs are the source of truth"));
    assert!(guide.contains("Runtime Docs As Source Of Truth"));
    assert!(readme.contains("Current runtime source of truth"));
}

#[test]
fn runtime_docs_declare_current_mvp_boundary() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let runtime_readme = std::fs::read_to_string(root.join("docs/runtime/README.md")).unwrap();
    let mvp_definition =
        std::fs::read_to_string(root.join("docs/runtime/mvp-definition.md")).unwrap();
    let implementation_status =
        std::fs::read_to_string(root.join("docs/runtime/implementation-status.md")).unwrap();
    let root_readme = std::fs::read_to_string(root.join("README.md")).unwrap();

    assert!(
        runtime_readme.contains("mvp-definition.md"),
        "runtime README should link to the MVP definition"
    );
    assert!(
        root_readme.contains("Current MVP"),
        "root README should expose the current MVP status"
    );
    assert!(
        implementation_status.contains("MVP Status"),
        "implementation status should expose the MVP status"
    );
    assert!(
        mvp_definition.contains("MVP reached"),
        "MVP definition should explicitly declare the reached state"
    );
    assert!(
        mvp_definition.contains("Out of scope"),
        "MVP definition should name exclusions"
    );
    assert!(
        mvp_definition.contains("Browser/Desktop"),
        "MVP definition should keep future workspace surfaces out of scope"
    );
}
