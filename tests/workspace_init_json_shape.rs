//! T-M1: byte-stable JSON wire-shape pin for
//! [`tome::workspace::InitOutcome`]. Constructs the struct via a literal
//! with known fields and asserts the exact `serde_json::to_string`
//! bytes. Catches accidental field-order / rename changes.

use std::path::PathBuf;
use tome::workspace::{InitOutcome, WorkspaceName};

#[cfg(unix)]
#[test]
fn init_outcome_json_wire_shape_is_byte_stable_unix() {
    let outcome = InitOutcome {
        name: WorkspaceName::parse("my-ws").unwrap(),
        workspace_dir: PathBuf::from("/tmp/tome/workspaces/my-ws"),
        inherited_catalogs: 3,
    };
    let json = serde_json::to_string(&outcome).expect("serialise");
    assert_eq!(
        json,
        r#"{"name":"my-ws","workspace_dir":"/tmp/tome/workspaces/my-ws","inherited_catalogs":3}"#,
    );
}

#[test]
fn init_outcome_json_field_order_is_pinned() {
    // Cross-platform variant: field order matters even when path
    // serialisation differs by OS. Match on the field-name sequence in
    // the emitted JSON.
    let outcome = InitOutcome {
        name: WorkspaceName::parse("x").unwrap(),
        workspace_dir: PathBuf::from("x"),
        inherited_catalogs: 0,
    };
    let json = serde_json::to_string(&outcome).expect("serialise");
    let name_idx = json.find("\"name\"").expect("name field present");
    let dir_idx = json
        .find("\"workspace_dir\"")
        .expect("workspace_dir present");
    let inherited_idx = json
        .find("\"inherited_catalogs\"")
        .expect("inherited_catalogs present");
    assert!(name_idx < dir_idx, "name must come before workspace_dir");
    assert!(
        dir_idx < inherited_idx,
        "workspace_dir must come before inherited_catalogs",
    );
}
