//! T-M1: byte-stable JSON wire-shape pin for
//! [`tome::workspace::RemoveOutcome`].

use std::path::PathBuf;
use tome::workspace::{RemoveOutcome, WorkspaceName};

#[cfg(unix)]
#[test]
fn remove_outcome_json_wire_shape_is_byte_stable_unix() {
    let outcome = RemoveOutcome {
        removed: WorkspaceName::parse("gone").unwrap(),
        bound_projects_torn_down: 1,
        catalog_caches_cleaned: vec!["https://example.com/a.git".into()],
        orphaned_paths: vec![PathBuf::from("/tmp/orphan")],
    };
    let json = serde_json::to_string(&outcome).expect("serialise");
    assert_eq!(
        json,
        r#"{"removed":"gone","bound_projects_torn_down":1,"catalog_caches_cleaned":["https://example.com/a.git"],"orphaned_paths":["/tmp/orphan"]}"#,
    );
}

#[test]
fn remove_outcome_empty_collections_render_as_empty_arrays() {
    let outcome = RemoveOutcome {
        removed: WorkspaceName::parse("gone").unwrap(),
        bound_projects_torn_down: 0,
        catalog_caches_cleaned: vec![],
        orphaned_paths: vec![],
    };
    let json = serde_json::to_string(&outcome).expect("serialise");
    assert!(
        json.contains("\"catalog_caches_cleaned\":[]"),
        "empty catalog_caches_cleaned should serialise as []: {json}",
    );
    assert!(
        json.contains("\"orphaned_paths\":[]"),
        "empty orphaned_paths should serialise as []: {json}",
    );
}
