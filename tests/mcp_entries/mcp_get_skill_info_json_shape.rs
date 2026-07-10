//! Byte-stable JSON wire-shape pin for the consolidated `get_skill`'s
//! METADATA-ONLY mode. #497 (was the standalone `get_skill_info` pin).
//!
//! Issue #497 folded `get_skill_info` into `get_skill` behind
//! `metadata_only: true`. The metadata-only response now serialises through the
//! shared `get_skill::Output`, so this file pins that shape:
//!
//! Metadata-only field order:
//!   catalog, plugin, name, kind, path, description, when_to_use,
//!   plugin_version, user_invocable, [resources], [prompt_name].
//! Every full-body field (`content` / flat `resources` array /
//! `substitutions_applied` / `resource_bodies`) is `Option`-gated and MUST be
//! absent here.
//!
//! Two snapshots are pinned:
//!
//! 1. Skill-kind: `resources` enumeration PRESENT (`files` array +
//!    `directories` BTreeMap-as-JSON-object).
//! 2. Command-kind: structured `resources` key entirely ABSENT (FR-083).
//!
//! Each snapshot is constructed directly from the public types so the test
//! doesn't need a staged workspace or the index — it pins the Serialize impl
//! shape. Any field rename, reorder, or default-flip will flip this test red.

use std::collections::BTreeMap;

use tome::mcp::tools::get_skill::{MetaWhenToUse, Output, ResourceEnumeration};
use tome::plugin::identity::EntryKind;

#[test]
fn metadata_wire_shape_for_skill_kind() {
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

    let out = Output {
        catalog: "midnight-expert".into(),
        plugin: "compact-dev".into(),
        name: "compact-circuits".into(),
        kind: EntryKind::Skill,
        path: "/abs/path/to/SKILL.md".into(),
        // Full-body-mode fields absent in metadata mode.
        content: None,
        resources_paths: None,
        substitutions_applied: None,
        resource_bodies: None,
        // Metadata-mode fields.
        description: Some("Full description text.".into()),
        when_to_use: MetaWhenToUse::Present("when guidance applies".into()),
        plugin_version: Some("1.4.0".into()),
        user_invocable: Some(false),
        resources: Some(ResourceEnumeration {
            files: vec!["/abs/skills/with-resources/config.json".into()],
            directories,
        }),
        prompt_name: None,
    };

    let json = serde_json::to_string(&out).expect("serialise");

    // Pin every field in document order. BTreeMap iteration is alphabetical, so
    // `examples` precedes `scripts`. `prompt_name` ABSENT (None +
    // skip_serializing_if). Every full-body field ABSENT.
    let expected = r#"{"catalog":"midnight-expert","plugin":"compact-dev","name":"compact-circuits","kind":"skill","path":"/abs/path/to/SKILL.md","description":"Full description text.","when_to_use":"when guidance applies","plugin_version":"1.4.0","user_invocable":false,"resources":{"files":["/abs/skills/with-resources/config.json"],"directories":{"examples":["/abs/skills/with-resources/examples/advanced.ts","/abs/skills/with-resources/examples/basic.ts"],"scripts":["/abs/skills/with-resources/scripts/audit.py","/abs/skills/with-resources/scripts/lint.py","/abs/skills/with-resources/scripts/build.sh","/abs/skills/with-resources/scripts/deploy.sh","/abs/skills/with-resources/scripts/test.sh","and 3 more"]}}}"#;

    assert_eq!(
        json, expected,
        "metadata-only skill-kind JSON wire shape drift (#497 consolidation)",
    );
}

#[test]
fn metadata_wire_shape_for_command_kind_omits_resources() {
    let out = Output {
        catalog: "midnight-expert".into(),
        plugin: "compact-dev".into(),
        name: "fix-issue".into(),
        kind: EntryKind::Command,
        path: "/abs/path/to/commands/fix-issue.md".into(),
        content: None,
        resources_paths: None,
        substitutions_applied: None,
        resource_bodies: None,
        description: Some("Fix a GitHub issue.".into()),
        // A command with no `when_to_use` guidance still serialises the field as
        // JSON `null` in metadata mode (matching the former get_skill_info shape).
        when_to_use: MetaWhenToUse::Null,
        plugin_version: Some("1.4.0".into()),
        user_invocable: Some(true),
        // FR-083: commands omit the structured resource enumeration entirely.
        resources: None,
        // #289: a user-invocable command carries its derived MCP prompt_name,
        // appended LAST.
        prompt_name: Some("compact-dev__fix-issue".into()),
    };

    let json = serde_json::to_string(&out).expect("serialise");

    // The structured `resources` key MUST be absent (FR-083). `when_to_use`
    // serialises as JSON `null`. `prompt_name` appended LAST.
    let expected = r#"{"catalog":"midnight-expert","plugin":"compact-dev","name":"fix-issue","kind":"command","path":"/abs/path/to/commands/fix-issue.md","description":"Fix a GitHub issue.","when_to_use":null,"plugin_version":"1.4.0","user_invocable":true,"prompt_name":"compact-dev__fix-issue"}"#;

    assert_eq!(
        json, expected,
        "metadata-only command-kind JSON wire shape drift — `resources` absent, `when_to_use` null, `prompt_name` appended LAST",
    );
}
