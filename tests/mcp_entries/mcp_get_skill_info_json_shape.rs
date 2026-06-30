//! Byte-stable JSON wire-shape pin for `get_skill_info`'s `SkillInfo`
//! response. Phase 5 / US4.a.
//!
//! Two snapshots are pinned:
//!
//! 1. Skill-kind: `resources` field PRESENT with the documented shape
//!    (`files` array + `directories` BTreeMap-as-JSON-object).
//! 2. Command-kind: `resources` key entirely ABSENT (FR-083 —
//!    `#[serde(skip_serializing_if = "Option::is_none")]`).
//!
//! Each snapshot is constructed directly from the public types so the
//! test doesn't need a staged workspace or the index — it pins the
//! Serialize impl shape, not the handler's behaviour (which the other
//! `mcp_get_skill_info.rs` tests cover end-to-end).
//!
//! The fields are listed in the order the contract documents; the
//! assertion compares the serialised string byte-for-byte against the
//! expected literal. Any field rename, reorder, or default-flip will
//! flip this test red.

use std::collections::BTreeMap;

use tome::mcp::tools::get_skill_info::{ResourceEnumeration, SkillInfo};
use tome::plugin::identity::EntryKind;

#[test]
fn skill_info_wire_shape_for_skill_kind() {
    let mut directories: BTreeMap<String, Vec<String>> = BTreeMap::new();
    directories.insert(
        "examples".into(),
        vec![
            "/abs/skills/with-resources/examples/advanced.ts".into(),
            "/abs/skills/with-resources/examples/basic.ts".into(),
        ],
    );
    directories.insert(
        "scripts".into(),
        vec![
            "/abs/skills/with-resources/scripts/audit.py".into(),
            "/abs/skills/with-resources/scripts/lint.py".into(),
            "/abs/skills/with-resources/scripts/build.sh".into(),
            "/abs/skills/with-resources/scripts/deploy.sh".into(),
            "/abs/skills/with-resources/scripts/test.sh".into(),
            "and 3 more".into(),
        ],
    );

    let info = SkillInfo {
        catalog: "midnight-expert".into(),
        plugin: "compact-dev".into(),
        name: "compact-circuits".into(),
        kind: EntryKind::Skill,
        path: "/abs/path/to/SKILL.md".into(),
        description: "Full description text.".into(),
        when_to_use: Some("when guidance applies".into()),
        plugin_version: "1.4.0".into(),
        user_invocable: false,
        resources: Some(ResourceEnumeration {
            files: vec!["/abs/skills/with-resources/config.json".into()],
            directories,
        }),
        // A non-invocable skill has no prompt — `prompt_name` is `None` and
        // MUST be omitted (#289 additive field, `skip_serializing_if`),
        // keeping the pre-#289 skill-kind wire shape byte-identical.
        prompt_name: None,
    };

    let json = serde_json::to_string(&info).expect("serialise");

    // Pin every field in document order. BTreeMap iteration is
    // alphabetical, so `examples` precedes `scripts` in the directories
    // object — the contract pins this. `prompt_name` is ABSENT (None +
    // skip_serializing_if).
    let expected = r#"{"catalog":"midnight-expert","plugin":"compact-dev","name":"compact-circuits","kind":"skill","path":"/abs/path/to/SKILL.md","description":"Full description text.","when_to_use":"when guidance applies","plugin_version":"1.4.0","user_invocable":false,"resources":{"files":["/abs/skills/with-resources/config.json"],"directories":{"examples":["/abs/skills/with-resources/examples/advanced.ts","/abs/skills/with-resources/examples/basic.ts"],"scripts":["/abs/skills/with-resources/scripts/audit.py","/abs/skills/with-resources/scripts/lint.py","/abs/skills/with-resources/scripts/build.sh","/abs/skills/with-resources/scripts/deploy.sh","/abs/skills/with-resources/scripts/test.sh","and 3 more"]}}}"#;

    assert_eq!(
        json, expected,
        "skill-kind JSON wire shape drift — check field renames, reorders, or default flips",
    );
}

#[test]
fn skill_info_wire_shape_for_command_kind_omits_resources() {
    let info = SkillInfo {
        catalog: "midnight-expert".into(),
        plugin: "compact-dev".into(),
        name: "fix-issue".into(),
        kind: EntryKind::Command,
        path: "/abs/path/to/commands/fix-issue.md".into(),
        description: "Fix a GitHub issue.".into(),
        when_to_use: None,
        plugin_version: "1.4.0".into(),
        user_invocable: true,
        resources: None,
        // #289: a user-invocable command carries its derived MCP `prompt_name`,
        // appended LAST so the additive field never reorders the pinned fields.
        prompt_name: Some("compact-dev__fix-issue".into()),
    };

    let json = serde_json::to_string(&info).expect("serialise");

    // The `resources` key MUST be absent (FR-083). Also pin the
    // `when_to_use: null` shape since the field is Option<String>
    // WITHOUT `skip_serializing_if` — it serialises as JSON `null`
    // rather than disappearing. `prompt_name` is appended LAST and present
    // (the command is user-invocable).
    let expected = r#"{"catalog":"midnight-expert","plugin":"compact-dev","name":"fix-issue","kind":"command","path":"/abs/path/to/commands/fix-issue.md","description":"Fix a GitHub issue.","when_to_use":null,"plugin_version":"1.4.0","user_invocable":true,"prompt_name":"compact-dev__fix-issue"}"#;

    assert_eq!(
        json, expected,
        "command-kind JSON wire shape drift — `resources` absent, `when_to_use` null, `prompt_name` appended LAST",
    );
}
