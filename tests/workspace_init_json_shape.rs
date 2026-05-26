//! Byte-stable JSON wire-shape pin for
//! [`tome::workspace::InitOutcome`]. Constructs the struct via a literal
//! with known fields and asserts the exact `serde_json::to_string`
//! bytes. Catches accidental field-order / rename changes.
//!
//! Wire shape is pinned by `contracts/workspace-commands.md` §
//! `tome workspace init`. Fields: `name`, `path`, `catalogs_inherited`,
//! `id` (in that order).

use std::path::PathBuf;
use tome::workspace::{InitOutcome, WorkspaceName};

#[cfg(unix)]
#[test]
fn init_outcome_json_wire_shape_is_byte_stable_unix() {
    let outcome = InitOutcome {
        name: WorkspaceName::parse("my-ws").unwrap(),
        path: PathBuf::from("/tmp/tome/workspaces/my-ws"),
        catalogs_inherited: 3,
        id: 7,
    };
    let json = serde_json::to_string(&outcome).expect("serialise");
    assert_eq!(
        json,
        r#"{"name":"my-ws","path":"/tmp/tome/workspaces/my-ws","catalogs_inherited":3,"id":7}"#,
    );
}

#[test]
fn init_outcome_json_field_order_is_pinned() {
    // Cross-platform variant: field order matters even when path
    // serialisation differs by OS. Match on the field-name sequence in
    // the emitted JSON.
    let outcome = InitOutcome {
        name: WorkspaceName::parse("x").unwrap(),
        path: PathBuf::from("x"),
        catalogs_inherited: 0,
        id: 1,
    };
    let json = serde_json::to_string(&outcome).expect("serialise");
    let name_idx = json.find("\"name\"").expect("name field present");
    let path_idx = json.find("\"path\"").expect("path present");
    let inherited_idx = json
        .find("\"catalogs_inherited\"")
        .expect("catalogs_inherited present");
    let id_idx = json.find("\"id\"").expect("id present");
    assert!(name_idx < path_idx, "name must come before path");
    assert!(
        path_idx < inherited_idx,
        "path must come before catalogs_inherited",
    );
    assert!(
        inherited_idx < id_idx,
        "catalogs_inherited must come before id",
    );
}
