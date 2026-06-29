//! T-M2 (US3 review) — byte-stable JSON serialisation pins for the
//! four `tome harness *` outcome types.
//!
//! The `--json` wire shape is consumed by editor integrations + CI
//! shells; any drift triggers a CI failure here before it reaches a
//! consumer.
//!
//! Unix-only: paths are serialised as-is, so the expected JSON embeds
//! Unix-style absolute paths.

use std::path::PathBuf;

use tome::commands::harness::bare::HarnessBareEntry;
use tome::commands::harness::info::{HarnessInfoOutcome, HarnessReference};
use tome::commands::harness::remove::HarnessRemoveOutcome;
use tome::commands::harness::use_::HarnessUseOutcome;

#[test]
fn harness_bare_entry_serialises_byte_stable() {
    let entry = HarnessBareEntry {
        name: "claude-code".to_string(),
        description: "Claude Code".to_string(),
        detected: true,
        rules_file: Some(PathBuf::from("/proj/CLAUDE.md")),
        mcp_config: PathBuf::from("/home/u/.claude/settings.json"),
    };
    let json = serde_json::to_string(&entry).expect("serialise");
    assert_eq!(
        json,
        r#"{"name":"claude-code","description":"Claude Code","detected":true,"rules_file":"/proj/CLAUDE.md","mcp_config":"/home/u/.claude/settings.json"}"#,
        "HarnessBareEntry wire shape drift",
    );
}

#[test]
fn harness_info_outcome_serialises_byte_stable_with_references() {
    let outcome = HarnessInfoOutcome {
        name: "codex".to_string(),
        description: "Codex".to_string(),
        detected: false,
        detected_path: PathBuf::from("/home/u/.codex"),
        rules_target: Some(PathBuf::from("/proj/AGENTS.md")),
        mcp_target: Some(PathBuf::from("/home/u/.codex/config.toml")),
        rules_block_present: Some(true),
        mcp_entry_present: Some(false),
        mcp_tome_owned: None,
        references: vec![HarnessReference {
            scope: "project".to_string(),
            via: Some("[global]".to_string()),
        }],
        // Phase 11 / US5 (T063): `None` → `skip_serializing_if` omits it, so
        // the pre-Phase-11 byte pin below is UNCHANGED.
        mcp_snippet: None,
        // Task 14: `None` → `skip_serializing_if` omits it; pin UNCHANGED.
        unrepresented_agents_notice: None,
    };
    let json = serde_json::to_string(&outcome).expect("serialise");
    assert_eq!(
        json,
        r#"{"name":"codex","description":"Codex","detected":false,"detected_path":"/home/u/.codex","rules_target":"/proj/AGENTS.md","mcp_target":"/home/u/.codex/config.toml","rules_block_present":true,"mcp_entry_present":false,"mcp_tome_owned":null,"references":[{"scope":"project","via":"[global]"}]}"#,
        "HarnessInfoOutcome wire shape drift",
    );
}

/// Phase 11 / US5 (T063): when `mcp_snippet` IS populated it is APPENDED LAST
/// (when `unrepresented_agents_notice` is absent, which is the common case for
/// native-supporting harnesses).
#[test]
fn harness_info_outcome_mcp_snippet_appends_last_when_present() {
    let outcome = HarnessInfoOutcome {
        name: "codex".to_string(),
        description: "Codex".to_string(),
        detected: false,
        detected_path: PathBuf::from("/home/u/.codex"),
        rules_target: None,
        mcp_target: None,
        rules_block_present: None,
        mcp_entry_present: None,
        mcp_tome_owned: None,
        references: vec![],
        mcp_snippet: Some("SNIP".to_string()),
        // Task 14: notice absent → omitted (skip_serializing_if), so
        // mcp_snippet remains last.
        unrepresented_agents_notice: None,
    };
    let json = serde_json::to_string(&outcome).expect("serialise");
    assert!(
        json.ends_with(r#""references":[],"mcp_snippet":"SNIP"}"#),
        "mcp_snippet must be the LAST field when notice absent; got: {json}",
    );
}

#[test]
fn harness_use_outcome_serialises_byte_stable() {
    let outcome = HarnessUseOutcome {
        scope: "global".to_string(),
        name: "cursor".to_string(),
        settings_path: PathBuf::from("/home/u/.tome/settings.toml"),
        list_changed: true,
        sync_ran: false,
        // Phase 11 / US5 (T064): `None` → omitted; pre-Phase-11 pin unchanged.
        mcp_notice: None,
    };
    let json = serde_json::to_string(&outcome).expect("serialise");
    assert_eq!(
        json,
        r#"{"scope":"global","name":"cursor","settings_path":"/home/u/.tome/settings.toml","list_changed":true,"sync_ran":false}"#,
        "HarnessUseOutcome wire shape drift",
    );
}

/// Phase 11 / US5 (T064): when `mcp_notice` IS populated it is APPENDED LAST.
#[test]
fn harness_use_outcome_mcp_notice_appends_last_when_present() {
    let outcome = HarnessUseOutcome {
        scope: "project".to_string(),
        name: "jetbrains-ai".to_string(),
        settings_path: PathBuf::from("/proj/.tome/config.toml"),
        list_changed: true,
        sync_ran: true,
        mcp_notice: Some("add it by hand".to_string()),
    };
    let json = serde_json::to_string(&outcome).expect("serialise");
    assert!(
        json.ends_with(r#""sync_ran":true,"mcp_notice":"add it by hand"}"#),
        "mcp_notice must be the LAST field; got: {json}",
    );
}

#[test]
fn harness_remove_outcome_serialises_byte_stable() {
    let outcome = HarnessRemoveOutcome {
        scope: "workspace".to_string(),
        name: "gemini".to_string(),
        settings_path: PathBuf::from("/home/u/.tome/workspaces/demo/settings.toml"),
        list_changed: false,
        sync_ran: false,
    };
    let json = serde_json::to_string(&outcome).expect("serialise");
    assert_eq!(
        json,
        r#"{"scope":"workspace","name":"gemini","settings_path":"/home/u/.tome/workspaces/demo/settings.toml","list_changed":false,"sync_ran":false}"#,
        "HarnessRemoveOutcome wire shape drift",
    );
}
