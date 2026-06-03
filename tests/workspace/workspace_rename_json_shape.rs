//! T-M1: byte-stable JSON wire-shape pin for
//! [`tome::workspace::RenameOutcome`].

use std::path::PathBuf;
use tome::workspace::{RenameOutcome, WorkspaceName};

#[cfg(unix)]
#[test]
fn rename_outcome_json_wire_shape_is_byte_stable_unix() {
    let outcome = RenameOutcome {
        old_name: WorkspaceName::parse("old").unwrap(),
        new_name: WorkspaceName::parse("new").unwrap(),
        bound_projects_updated: 2,
        workspace_dir: PathBuf::from("/tmp/tome/workspaces/new"),
    };
    let json = serde_json::to_string(&outcome).expect("serialise");
    assert_eq!(
        json,
        r#"{"old_name":"old","new_name":"new","bound_projects_updated":2,"workspace_dir":"/tmp/tome/workspaces/new"}"#,
    );
}

#[test]
fn rename_outcome_field_order_is_pinned() {
    let outcome = RenameOutcome {
        old_name: WorkspaceName::parse("a").unwrap(),
        new_name: WorkspaceName::parse("b").unwrap(),
        bound_projects_updated: 0,
        workspace_dir: PathBuf::from("x"),
    };
    let json = serde_json::to_string(&outcome).expect("serialise");
    let old_idx = json.find("\"old_name\"").unwrap();
    let new_idx = json.find("\"new_name\"").unwrap();
    let updated_idx = json.find("\"bound_projects_updated\"").unwrap();
    let dir_idx = json.find("\"workspace_dir\"").unwrap();
    assert!(old_idx < new_idx);
    assert!(new_idx < updated_idx);
    assert!(updated_idx < dir_idx);
}
