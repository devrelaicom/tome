//! T-M6: byte-stable JSON wire-shape pin for [`tome::harness::sync`]'s
//! [`SyncOutcome`] envelope. This is the most-consumed Phase 4 JSON
//! envelope — `tome workspace use`, `tome harness use`, and
//! `tome harness sync` all emit it directly. If the shape drifts, every
//! downstream `jq` consumer breaks; pin it here.
//!
//! Wire shape is pinned by `contracts/sync-algorithm.md` §"JSON output"
//! plus `contracts/harness-commands.md` § `sync`.
//!
//! Covered:
//! - `SyncOutcome` field order: `added` / `updated` / `removed` /
//!   `leave_alones` / `decisions`.
//! - `SyncChange` field order: `harness` / `subsystem` / `path`.
//! - `SyncSubsystem` snake_case: `rules` / `mcp` / `agents`.
//! - `HarnessDecision` field order: `harness` / `in_effective_list` /
//!   `rules_action` / `mcp_action` / `agents_action` (Phase 6 / US1 adds
//!   `agents_action` LAST so the existing prefix order is unchanged).
//! - `Action` snake_case: `created` / `updated` / `removed` /
//!   `left_alone`.

use std::path::PathBuf;
use tome::harness::sync::{Action, HarnessDecision, SyncChange, SyncOutcome, SyncSubsystem};

#[cfg(unix)]
#[test]
fn sync_outcome_json_wire_shape_is_byte_stable_unix() {
    let outcome = SyncOutcome {
        added: vec![SyncChange {
            harness: "claude-code".to_owned(),
            subsystem: SyncSubsystem::Rules,
            path: PathBuf::from("/proj/.claude/CLAUDE.md"),
        }],
        updated: vec![SyncChange {
            harness: "codex".to_owned(),
            subsystem: SyncSubsystem::Mcp,
            path: PathBuf::from("/home/u/.codex/config.toml"),
        }],
        removed: vec![SyncChange {
            harness: "cursor".to_owned(),
            subsystem: SyncSubsystem::Rules,
            path: PathBuf::from("/proj/AGENTS.md"),
        }],
        leave_alones: 2,
        decisions: vec![HarnessDecision {
            harness: "claude-code".to_owned(),
            in_effective_list: true,
            rules_action: Action::Created,
            mcp_action: Action::LeftAlone,
            agents_action: Action::Created,
        }],
    };
    let json = serde_json::to_string(&outcome).expect("serialise");
    assert_eq!(
        json,
        r#"{"added":[{"harness":"claude-code","subsystem":"rules","path":"/proj/.claude/CLAUDE.md"}],"updated":[{"harness":"codex","subsystem":"mcp","path":"/home/u/.codex/config.toml"}],"removed":[{"harness":"cursor","subsystem":"rules","path":"/proj/AGENTS.md"}],"leave_alones":2,"decisions":[{"harness":"claude-code","in_effective_list":true,"rules_action":"created","mcp_action":"left_alone","agents_action":"created"}]}"#,
    );
}

#[test]
fn sync_outcome_empty_default_serialises_clean() {
    let outcome = SyncOutcome::default();
    let json = serde_json::to_string(&outcome).unwrap();
    assert_eq!(
        json,
        r#"{"added":[],"updated":[],"removed":[],"leave_alones":0,"decisions":[]}"#,
    );
}

#[test]
fn sync_outcome_field_order_is_pinned() {
    let outcome = SyncOutcome::default();
    let json = serde_json::to_string(&outcome).unwrap();
    let added_idx = json.find("\"added\"").expect("added present");
    let updated_idx = json.find("\"updated\"").expect("updated present");
    let removed_idx = json.find("\"removed\"").expect("removed present");
    let leave_idx = json.find("\"leave_alones\"").expect("leave_alones present");
    let decisions_idx = json.find("\"decisions\"").expect("decisions present");
    assert!(added_idx < updated_idx, "added before updated");
    assert!(updated_idx < removed_idx, "updated before removed");
    assert!(removed_idx < leave_idx, "removed before leave_alones");
    assert!(leave_idx < decisions_idx, "leave_alones before decisions");
}

#[test]
fn sync_subsystem_snake_case_wire_shape() {
    assert_eq!(
        serde_json::to_string(&SyncSubsystem::Rules).unwrap(),
        "\"rules\""
    );
    assert_eq!(
        serde_json::to_string(&SyncSubsystem::Mcp).unwrap(),
        "\"mcp\""
    );
    assert_eq!(
        serde_json::to_string(&SyncSubsystem::Agents).unwrap(),
        "\"agents\""
    );
}

#[test]
fn action_snake_case_wire_shape() {
    assert_eq!(
        serde_json::to_string(&Action::Created).unwrap(),
        "\"created\""
    );
    assert_eq!(
        serde_json::to_string(&Action::Updated).unwrap(),
        "\"updated\""
    );
    assert_eq!(
        serde_json::to_string(&Action::Removed).unwrap(),
        "\"removed\""
    );
    // The `LeftAlone` variant snake-cases to `left_alone` per the
    // contract; pin explicitly because the default snake_case rule
    // could plausibly be misread as `leftalone` by a future reviewer.
    assert_eq!(
        serde_json::to_string(&Action::LeftAlone).unwrap(),
        "\"left_alone\""
    );
}

#[test]
fn harness_decision_field_order_is_pinned() {
    let decision = HarnessDecision {
        harness: "x".to_owned(),
        in_effective_list: false,
        rules_action: Action::Removed,
        mcp_action: Action::LeftAlone,
        agents_action: Action::LeftAlone,
    };
    let json = serde_json::to_string(&decision).unwrap();
    assert_eq!(
        json,
        r#"{"harness":"x","in_effective_list":false,"rules_action":"removed","mcp_action":"left_alone","agents_action":"left_alone"}"#,
    );
}

#[test]
fn sync_change_field_order_is_pinned() {
    let change = SyncChange {
        harness: "h".to_owned(),
        subsystem: SyncSubsystem::Rules,
        path: PathBuf::from("p"),
    };
    let json = serde_json::to_string(&change).unwrap();
    let harness_idx = json.find("\"harness\"").unwrap();
    let subsystem_idx = json.find("\"subsystem\"").unwrap();
    let path_idx = json.find("\"path\"").unwrap();
    assert!(harness_idx < subsystem_idx, "harness before subsystem");
    assert!(subsystem_idx < path_idx, "subsystem before path");
}
