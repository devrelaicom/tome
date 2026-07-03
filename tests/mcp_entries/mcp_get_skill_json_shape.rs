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

use tome::mcp::tools::get_skill::{Output, ResourceBody};
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
        // #331: the default (rendered) mode reports `substitutions_applied:
        // true`. Always present (non-`Option`), appended LAST.
        substitutions_applied: true,
        // #333: `include_resource_bodies` was NOT requested → `resource_bodies`
        // is `None` and MUST be omitted (skip_serializing_if), so the flag-off
        // wire shape is byte-identical to pre-#333.
        resource_bodies: None,
    };

    let json = serde_json::to_string(&out).expect("serialise");

    // Document order: content, path, resources, kind, [prompt_name omitted],
    // substitutions_applied, [resource_bodies omitted]. `kind` is lowercase via
    // `#[serde(rename_all = "lowercase")]` on `EntryKind`. `prompt_name` is
    // ABSENT (None + skip_serializing_if). `substitutions_applied` is the
    // additive #331 key. `resource_bodies` is ABSENT (None + skip_serializing_if,
    // #333) — the flag-off shape is byte-identical to pre-#333.
    let expected = r#"{"content":"rendered skill body","path":"/abs/path/to/SKILL.md","resources":["/abs/path/to/examples/basic.ts"],"kind":"skill","substitutions_applied":true}"#;

    assert_eq!(
        json, expected,
        "get_skill skill-kind JSON wire shape drift — `kind` must be present (additive #289), `prompt_name` absent, `substitutions_applied` present (additive #331), `resource_bodies` absent (additive #333, flag off)",
    );
}

#[test]
fn get_skill_output_wire_shape_for_raw_mode() {
    // #331: `raw: true` preserves literal `${TOME_*}` tokens and reports
    // `substitutions_applied: false`. Wire shape is otherwise identical — the
    // only difference is the boolean value of the always-present key.
    let out = Output {
        content: "raw body with ${TOME_PROJECT_DIR} preserved".into(),
        path: "/abs/path/to/SKILL.md".into(),
        resources: vec![],
        kind: EntryKind::Skill,
        prompt_name: None,
        substitutions_applied: false,
        // #333: not requested → absent, byte-identical to pre-#333.
        resource_bodies: None,
    };

    let json = serde_json::to_string(&out).expect("serialise");

    let expected = r#"{"content":"raw body with ${TOME_PROJECT_DIR} preserved","path":"/abs/path/to/SKILL.md","resources":[],"kind":"skill","substitutions_applied":false}"#;

    assert_eq!(
        json, expected,
        "get_skill raw-mode JSON wire shape drift — literal token preserved in `content`, `substitutions_applied` must be `false`",
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
        // #331: rendered mode; `substitutions_applied` follows `prompt_name`.
        substitutions_applied: true,
        // #333: commands have no resource directory → `resource_bodies` stays
        // `None`/absent even had the flag been passed.
        resource_bodies: None,
    };

    let json = serde_json::to_string(&out).expect("serialise");

    let expected = r#"{"content":"run the deploy","path":"/abs/path/to/commands/deploy.md","resources":[],"kind":"command","prompt_name":"plug__deploy","substitutions_applied":true}"#;

    assert_eq!(
        json, expected,
        "get_skill command-kind JSON wire shape drift — `kind` lowercase `command`, `prompt_name` then `substitutions_applied` appended LAST, `resource_bodies` absent (#333)",
    );
}

#[test]
fn get_skill_output_wire_shape_with_inlined_resource_bodies() {
    // #333: when `include_resource_bodies` was requested and at least one
    // resource fit the byte budget, `resource_bodies` is PRESENT as an array of
    // `{ path, content }`, appended LAST (after `substitutions_applied`). Each
    // `path` also appears in `resources` — `resource_bodies` is a parallel VIEW,
    // not a replacement.
    let out = Output {
        content: "rendered skill body".into(),
        path: "/abs/path/to/SKILL.md".into(),
        resources: vec![
            "/abs/path/to/examples/basic.ts".into(),
            "/abs/path/to/data/blob.bin".into(),
        ],
        kind: EntryKind::Skill,
        prompt_name: None,
        substitutions_applied: true,
        // Only the text resource was inlined; the binary one is omitted here but
        // still present in `resources` above.
        resource_bodies: Some(vec![ResourceBody {
            path: "/abs/path/to/examples/basic.ts".into(),
            content: "export const x = 1;\n".into(),
        }]),
    };

    let json = serde_json::to_string(&out).expect("serialise");

    // Order: content, path, resources, kind, [prompt_name omitted],
    // substitutions_applied, resource_bodies. Each `resource_bodies` element is
    // `{ "path": ..., "content": ... }` in field order.
    let expected = r#"{"content":"rendered skill body","path":"/abs/path/to/SKILL.md","resources":["/abs/path/to/examples/basic.ts","/abs/path/to/data/blob.bin"],"kind":"skill","substitutions_applied":true,"resource_bodies":[{"path":"/abs/path/to/examples/basic.ts","content":"export const x = 1;\n"}]}"#;

    assert_eq!(
        json, expected,
        "get_skill inlined-resource-bodies JSON wire shape drift — `resource_bodies` appended LAST as an array of path/content objects",
    );
}
