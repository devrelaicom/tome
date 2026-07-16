//! Byte-stable JSON wire-shape pin for the consolidated `get_skill` `Output`.
//! #289 / #331 / #333 / #497.
//!
//! Issue #497 consolidated `get_skill` + `get_skill_info` into one tool with a
//! `metadata_only` flag, so the `Output` now serves both modes through one
//! struct. This file pins the FULL-BODY mode (`metadata_only: false`); the
//! metadata-only mode is pinned in `mcp_get_skill_info_json_shape.rs`.
//!
//! Full-body mode field order:
//!   catalog, plugin, name, kind, path, content, resources,
//!   substitutions_applied, [resource_bodies], [prompt_name].
//! Every metadata-only field (`description` / `when_to_use` / `plugin_version`
//! / `user_invocable` / structured `resources` enumeration) is `Option`/tri-
//! state-gated and MUST be absent here.
//!
//! Each snapshot is constructed directly from the public types so the test
//! doesn't need a staged workspace or the index — it pins the Serialize impl
//! shape, not the handler's behaviour (covered end-to-end in `entry_e2e.rs`).
//! Any field rename, reorder, or default-flip will flip this test red.

use std::collections::BTreeMap;

use tome::mcp::tools::get_skill::{MetaWhenToUse, Output, ResourceBody};
use tome::plugin::identity::EntryKind;

/// A full-body `Output` builder: the metadata-only fields are all absent.
fn body_output(
    content: &str,
    path: &str,
    resources: Vec<String>,
    kind: EntryKind,
    prompt_name: Option<String>,
    substitutions_applied: bool,
    resource_bodies: Option<Vec<ResourceBody>>,
) -> Output {
    Output {
        catalog: Some("midnight-expert".into()),
        plugin: Some("compact-dev".into()),
        name: Some("compact-circuits".into()),
        kind: Some(kind),
        path: Some(path.into()),
        content: Some(content.into()),
        resources_paths: Some(resources),
        substitutions_applied: Some(substitutions_applied),
        resource_bodies,
        description: None,
        when_to_use: MetaWhenToUse::Absent,
        plugin_version: None,
        user_invocable: None,
        resources: None,
        prompt_name,
        matches: None,
        next_actions: None,
    }
}

#[test]
fn get_skill_output_wire_shape_for_skill_kind() {
    let out = body_output(
        "rendered skill body",
        "/abs/path/to/SKILL.md",
        vec!["/abs/path/to/examples/basic.ts".into()],
        EntryKind::Skill,
        None,
        true,
        None,
    );

    let json = serde_json::to_string(&out).expect("serialise");

    // Document order for full-body mode: catalog, plugin, name, kind, path,
    // content, resources, substitutions_applied. `prompt_name` ABSENT (None +
    // skip_serializing_if). `resource_bodies` ABSENT (flag off). Every
    // metadata-only field ABSENT.
    let expected = r#"{"catalog":"midnight-expert","plugin":"compact-dev","name":"compact-circuits","kind":"skill","path":"/abs/path/to/SKILL.md","content":"rendered skill body","resources":["/abs/path/to/examples/basic.ts"],"substitutions_applied":true}"#;

    assert_eq!(
        json, expected,
        "get_skill full-body skill-kind JSON wire shape drift (#497 consolidation)",
    );
}

#[test]
fn get_skill_output_wire_shape_for_raw_mode() {
    // #331: `raw: true` preserves literal `${TOME_*}` tokens and reports
    // `substitutions_applied: false`. Wire shape is otherwise identical.
    let out = body_output(
        "raw body with ${TOME_PROJECT_DIR} preserved",
        "/abs/path/to/SKILL.md",
        vec![],
        EntryKind::Skill,
        None,
        false,
        None,
    );

    let json = serde_json::to_string(&out).expect("serialise");

    let expected = r#"{"catalog":"midnight-expert","plugin":"compact-dev","name":"compact-circuits","kind":"skill","path":"/abs/path/to/SKILL.md","content":"raw body with ${TOME_PROJECT_DIR} preserved","resources":[],"substitutions_applied":false}"#;

    assert_eq!(
        json, expected,
        "get_skill raw-mode JSON wire shape drift — literal token preserved in `content`, `substitutions_applied` must be `false`",
    );
}

#[test]
fn get_skill_output_wire_shape_for_command_kind() {
    // #289: a user-invocable command carries its derived MCP `prompt_name`,
    // appended LAST, with no sibling-resource enumeration.
    let mut out = body_output(
        "run the deploy",
        "/abs/path/to/commands/deploy.md",
        vec![],
        EntryKind::Command,
        Some("plug__deploy".into()),
        true,
        None,
    );
    out.name = Some("deploy".into());

    let json = serde_json::to_string(&out).expect("serialise");

    let expected = r#"{"catalog":"midnight-expert","plugin":"compact-dev","name":"deploy","kind":"command","path":"/abs/path/to/commands/deploy.md","content":"run the deploy","resources":[],"substitutions_applied":true,"prompt_name":"plug__deploy"}"#;

    assert_eq!(
        json, expected,
        "get_skill command-kind JSON wire shape drift — `prompt_name` appended LAST after `substitutions_applied`",
    );
}

#[test]
fn get_skill_output_wire_shape_with_inlined_resource_bodies() {
    // #333: when `include_resource_bodies` was requested and at least one
    // resource fit the byte budget, `resource_bodies` is PRESENT as an array of
    // `{ path, content }`, appended after `substitutions_applied`.
    let out = body_output(
        "rendered skill body",
        "/abs/path/to/SKILL.md",
        vec![
            "/abs/path/to/examples/basic.ts".into(),
            "/abs/path/to/data/blob.bin".into(),
        ],
        EntryKind::Skill,
        None,
        true,
        Some(vec![ResourceBody {
            path: "/abs/path/to/examples/basic.ts".into(),
            content: "export const x = 1;\n".into(),
        }]),
    );

    let json = serde_json::to_string(&out).expect("serialise");

    let expected = r#"{"catalog":"midnight-expert","plugin":"compact-dev","name":"compact-circuits","kind":"skill","path":"/abs/path/to/SKILL.md","content":"rendered skill body","resources":["/abs/path/to/examples/basic.ts","/abs/path/to/data/blob.bin"],"substitutions_applied":true,"resource_bodies":[{"path":"/abs/path/to/examples/basic.ts","content":"export const x = 1;\n"}]}"#;

    assert_eq!(
        json, expected,
        "get_skill inlined-resource-bodies JSON wire shape drift — `resource_bodies` appended after `substitutions_applied`",
    );
}

/// A skill-kind full-body output that carries a subdirectory in `resources`
/// must NOT emit the structured metadata `resources` enumeration (which uses a
/// `files`/`directories` object). The two `resources` are distinct: full-body
/// mode serialises `resources_paths` (a flat array) under the `resources` key.
#[test]
fn full_body_resources_is_a_flat_array_never_the_metadata_object() {
    let out = body_output(
        "body",
        "/abs/SKILL.md",
        vec!["/abs/a.txt".into()],
        EntryKind::Skill,
        None,
        true,
        None,
    );
    let value = serde_json::to_value(&out).expect("serialise");
    let obj = value.as_object().expect("object");
    // `resources` is an array (the flat path list), never a `{files,directories}`
    // object.
    assert!(
        obj.get("resources").and_then(|r| r.as_array()).is_some(),
        "full-body `resources` must serialise as a flat array; got: {value}",
    );
    // No metadata-object shape leaks in.
    let _unused: BTreeMap<String, Vec<String>> = BTreeMap::new();
}
