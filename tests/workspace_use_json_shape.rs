//! Byte-stable JSON serialisation pin for [`BindOutcome`] (Phase 4 /
//! US1.d-2a — T-M7 from the reviewer pass).
//!
//! `tome workspace use --json` is consumed by editors and CI shells
//! parsing the structured outcome. The wire shape MUST be stable across
//! refactors; serde's struct-field-rename surface is wide enough that a
//! casual rename or default-shadowing edit can silently change the JSON
//! payload. This test pins the serialised form to a known byte string
//! so any drift triggers a CI failure.
//!
//! Unix-only (`#[cfg(unix)]`): paths are serialised as-is, so the
//! expected JSON embeds a Unix-style absolute path. The Windows port
//! (if it ever happens) gets its own gated test.

#![cfg(unix)]

use std::path::PathBuf;

use tome::workspace::WorkspaceName;
use tome::workspace::binding::BindOutcome;

fn ws(name: &str) -> WorkspaceName {
    WorkspaceName::parse(name).expect("parse workspace name")
}

// ---------------------------------------------------------------------------
// 1. First bind: no prior workspace, marker created.
// ---------------------------------------------------------------------------

#[test]
fn first_bind_outcome_serialises_to_byte_stable_json() {
    let outcome = BindOutcome {
        workspace: ws("demo"),
        project_root: PathBuf::from("/tmp/proj"),
        created_marker: true,
        rebind_from: None,
        sync: None,
    };
    let json = serde_json::to_string(&outcome).expect("serialise");
    assert_eq!(
        json,
        r#"{"workspace":"demo","project_root":"/tmp/proj","created_marker":true,"rebind_from":null}"#,
        "BindOutcome wire shape drift"
    );
}

// ---------------------------------------------------------------------------
// 2. Re-bind to existing workspace: marker existed, no rebind.
// ---------------------------------------------------------------------------

#[test]
fn rebind_same_workspace_outcome_serialises_to_byte_stable_json() {
    let outcome = BindOutcome {
        workspace: ws("demo"),
        project_root: PathBuf::from("/tmp/proj"),
        created_marker: false,
        rebind_from: None,
        sync: None,
    };
    let json = serde_json::to_string(&outcome).expect("serialise");
    assert_eq!(
        json,
        r#"{"workspace":"demo","project_root":"/tmp/proj","created_marker":false,"rebind_from":null}"#,
        "BindOutcome wire shape drift"
    );
}

// ---------------------------------------------------------------------------
// 3. Rebind from a different workspace: rebind_from is populated.
// ---------------------------------------------------------------------------

#[test]
fn rebind_from_other_workspace_outcome_serialises_to_byte_stable_json() {
    let outcome = BindOutcome {
        workspace: ws("ws-b"),
        project_root: PathBuf::from("/tmp/proj"),
        created_marker: false,
        rebind_from: Some(ws("ws-a")),
        sync: None,
    };
    let json = serde_json::to_string(&outcome).expect("serialise");
    assert_eq!(
        json,
        r#"{"workspace":"ws-b","project_root":"/tmp/proj","created_marker":false,"rebind_from":"ws-a"}"#,
        "BindOutcome wire shape drift"
    );
}

// ---------------------------------------------------------------------------
// 4. With sync present: ensures the `sync` field renders inside the
//    envelope rather than being silently dropped.
// ---------------------------------------------------------------------------

#[test]
fn outcome_with_sync_serialises_field_present() {
    use tome::harness::sync::SyncOutcome;
    let outcome = BindOutcome {
        workspace: ws("demo"),
        project_root: PathBuf::from("/tmp/proj"),
        created_marker: true,
        rebind_from: None,
        sync: Some(SyncOutcome::default()),
    };
    let json = serde_json::to_string(&outcome).expect("serialise");
    // The SyncOutcome::default() produces a known shape — assert the
    // `sync` field is present and the surrounding envelope is intact.
    assert!(
        json.contains(r#""sync":{"#),
        "sync field must serialise when present; got: {json}",
    );
    assert!(
        json.contains(r#""workspace":"demo""#),
        "workspace must still serialise alongside sync; got: {json}"
    );
}
