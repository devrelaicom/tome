//! Byte-stable JSON wire-shape pin for `search_skills`'s `Output`
//! / `SkillMatch` Phase 5 shape. Phase 5 / US4.c.
//!
//! Two snapshots are pinned:
//!
//! 1. `SkillMatch` for a skill-kind hit — `kind: "skill"` lowercased per
//!    `EntryKind`'s `#[serde(rename_all = "lowercase")]` derive, all
//!    documented fields present in document order per
//!    `contracts/mcp-tools-p5.md` § `search_skills` result element.
//! 2. `SkillMatch` for a command-kind hit — `kind: "command"` to prove
//!    the discriminator round-trips both variants.
//!
//! Each snapshot is constructed directly from the public types so the
//! test doesn't need a staged workspace or the index — it pins the
//! Serialize impl shape, not the handler's behaviour (which the other
//! `mcp_search_skills_truncation.rs` tests cover end-to-end).
//!
//! Any field rename, reorder, default flip, or accidental
//! `#[serde(skip_serializing_if = ...)]` addition will flip this test
//! red.

use tome::mcp::tools::search_skills::{Output, SkillMatch};
use tome::plugin::identity::EntryKind;

#[test]
fn skill_match_wire_shape_for_skill_kind() {
    let m = SkillMatch {
        catalog: "midnight-expert".into(),
        plugin: "compact-dev".into(),
        name: "compact-circuits".into(),
        kind: EntryKind::Skill,
        description: "Truncated description body.".into(),
        plugin_version: "1.4.0".into(),
        path: "/abs/path/to/SKILL.md".into(),
        score: 0.87,
    };
    let out = Output { matches: vec![m] };

    let json = serde_json::to_string(&out).expect("serialise");

    // Document order: catalog, plugin, name, kind, description,
    // plugin_version, path, score. `kind` is lowercase via
    // `#[serde(rename_all = "lowercase")]` on `EntryKind`.
    let expected = r#"{"matches":[{"catalog":"midnight-expert","plugin":"compact-dev","name":"compact-circuits","kind":"skill","description":"Truncated description body.","plugin_version":"1.4.0","path":"/abs/path/to/SKILL.md","score":0.87}]}"#;

    assert_eq!(
        json, expected,
        "search_skills skill-kind JSON wire shape drift — check field renames, reorders, or default flips",
    );
}

#[test]
fn skill_match_wire_shape_for_command_kind() {
    let m = SkillMatch {
        catalog: "midnight-expert".into(),
        plugin: "compact-dev".into(),
        name: "fix-issue".into(),
        kind: EntryKind::Command,
        description: "Fix a GitHub issue.".into(),
        plugin_version: "1.4.0".into(),
        path: "/abs/path/to/commands/fix-issue.md".into(),
        score: 0.42,
    };
    let out = Output { matches: vec![m] };

    let json = serde_json::to_string(&out).expect("serialise");

    let expected = r#"{"matches":[{"catalog":"midnight-expert","plugin":"compact-dev","name":"fix-issue","kind":"command","description":"Fix a GitHub issue.","plugin_version":"1.4.0","path":"/abs/path/to/commands/fix-issue.md","score":0.42}]}"#;

    assert_eq!(
        json, expected,
        "search_skills command-kind JSON wire shape drift — `kind` must serialise as lowercase `command`",
    );
}

#[test]
fn empty_matches_wire_shape() {
    let out = Output { matches: vec![] };
    let json = serde_json::to_string(&out).expect("serialise");
    assert_eq!(
        json, r#"{"matches":[]}"#,
        "empty matches must serialise as empty JSON array, not null or omitted",
    );
}
