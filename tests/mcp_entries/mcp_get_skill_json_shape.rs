//! Byte-stable JSON wire-shape pin for `get_skill`'s `Output`. #289.
//!
//! `get_skill_info` and `search_skills` each have a shape pin; `get_skill`
//! previously had none. #289 made `get_skill` additive: a non-`Option` `kind`
//! key now appears on EVERY result (skill-kind included), and an optional
//! `prompt_name` appears for user-invocable entries (omitted otherwise via
//! `#[serde(skip_serializing_if = "Option::is_none")]`).
//!
//! Two snapshots are pinned:
//!
//! 1. Skill-kind, non-invocable: `kind: "skill"` PRESENT (the additive #289
//!    key), `prompt_name` ABSENT. This is the common skill case — the pin
//!    documents that the pre-#289 fields keep their order and the only change
//!    is the appended `kind` key.
//! 2. Command-kind, invocable: `kind: "command"` + `prompt_name` PRESENT,
//!    appended LAST, with empty `resources`.
//!
//! Each snapshot is constructed directly from the public types so the test
//! doesn't need a staged workspace or the index — it pins the Serialize impl
//! shape, not the handler's behaviour (covered end-to-end in `entry_e2e.rs`).
//! Any field rename, reorder, or default-flip will flip this test red.

use tome::mcp::tools::get_skill::Output;
use tome::plugin::identity::EntryKind;

#[test]
fn get_skill_output_wire_shape_for_skill_kind() {
    let out = Output {
        content: "rendered skill body".into(),
        path: "/abs/path/to/SKILL.md".into(),
        resources: vec!["/abs/path/to/examples/basic.ts".into()],
        kind: EntryKind::Skill,
        // A non-invocable skill has no prompt — `prompt_name` is `None` and
        // MUST be omitted; only the additive `kind` key is added vs pre-#289.
        prompt_name: None,
    };

    let json = serde_json::to_string(&out).expect("serialise");

    // Document order: content, path, resources, kind, [prompt_name omitted].
    // `kind` is lowercase via `#[serde(rename_all = "lowercase")]` on
    // `EntryKind`. `prompt_name` is ABSENT (None + skip_serializing_if).
    let expected = r#"{"content":"rendered skill body","path":"/abs/path/to/SKILL.md","resources":["/abs/path/to/examples/basic.ts"],"kind":"skill"}"#;

    assert_eq!(
        json, expected,
        "get_skill skill-kind JSON wire shape drift — `kind` must be present (additive #289), `prompt_name` absent",
    );
}

#[test]
fn get_skill_output_wire_shape_for_command_kind() {
    // #289: a user-invocable command resolved by get_skill carries its derived
    // MCP `prompt_name`, appended LAST, with no sibling-resource enumeration.
    let out = Output {
        content: "run the deploy".into(),
        path: "/abs/path/to/commands/deploy.md".into(),
        resources: vec![],
        kind: EntryKind::Command,
        prompt_name: Some("plug__deploy".into()),
    };

    let json = serde_json::to_string(&out).expect("serialise");

    let expected = r#"{"content":"run the deploy","path":"/abs/path/to/commands/deploy.md","resources":[],"kind":"command","prompt_name":"plug__deploy"}"#;

    assert_eq!(
        json, expected,
        "get_skill command-kind JSON wire shape drift — `kind` lowercase `command`, `prompt_name` appended LAST",
    );
}
