//! T-M1: byte-stable JSON wire-shape pin for
//! [`tome::workspace::RegenSummaryOutcome`].

use tome::workspace::{RegenSummaryOutcome, WorkspaceName};

#[test]
fn regen_summary_outcome_json_wire_shape_is_byte_stable() {
    let outcome = RegenSummaryOutcome {
        workspace: WorkspaceName::parse("ws").unwrap(),
        short_chars: 42,
        long_chars: 1337,
        bound_projects_synced: 3,
    };
    let json = serde_json::to_string(&outcome).expect("serialise");
    assert_eq!(
        json,
        r#"{"workspace":"ws","short_chars":42,"long_chars":1337,"bound_projects_synced":3}"#,
    );
}

#[test]
fn regen_summary_outcome_field_order_is_pinned() {
    let outcome = RegenSummaryOutcome {
        workspace: WorkspaceName::parse("ws").unwrap(),
        short_chars: 0,
        long_chars: 0,
        bound_projects_synced: 0,
    };
    let json = serde_json::to_string(&outcome).expect("serialise");
    let ws_idx = json.find("\"workspace\"").unwrap();
    let short_idx = json.find("\"short_chars\"").unwrap();
    let long_idx = json.find("\"long_chars\"").unwrap();
    let synced_idx = json.find("\"bound_projects_synced\"").unwrap();
    assert!(ws_idx < short_idx);
    assert!(short_idx < long_idx);
    assert!(long_idx < synced_idx);
}
