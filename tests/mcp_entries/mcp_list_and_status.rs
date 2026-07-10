//! Issue #497 — the new read-only discovery/introspection MCP tools
//! (`list_plugins`, `list_catalogs`, `status`) plus the consolidated
//! `get_skill` `metadata_only` mode, end-to-end through the in-process MCP
//! harness against a staged + indexed workspace (StubEmbedder — no ONNX).

use tome::mcp::tools::{get_skill, list_catalogs, list_plugins, status};
use tome::plugin::identity::EntryKind;

use crate::common::mcp_harness::StagedWorkspace;

const SKILL: &str = "---\nname: alpha\ndescription: The alpha skill.\nwhen_to_use: When alpha applies.\n---\nAlpha body.\n";
const COMMAND: &str = "---\nname: run-it\ndescription: Run the thing.\n---\nDo $ARGUMENTS\n";

// ---------------------------------------------------------------------------
// list_plugins
// ---------------------------------------------------------------------------

#[test]
fn list_plugins_enumerates_enabled_plugin_and_its_entries() {
    let staged = StagedWorkspace::stage(&[("alpha", SKILL)], &[("run-it", COMMAND)]);
    let harness = staged.harness();

    let out = harness
        .call_list_plugins(list_plugins::Input {
            catalog: None,
            enabled_only: true,
            kind: None,
        })
        .expect("list_plugins ok");

    assert_eq!(out.workspace, "global");
    assert_eq!(out.plugins.len(), 1, "one staged plugin");
    let p = &out.plugins[0];
    assert_eq!(p.catalog, "acme");
    assert_eq!(p.plugin, "plug");
    assert_eq!(p.version.as_deref(), Some("1.0.0"));
    assert_eq!(p.enabled_entries, 2, "one skill + one command enabled");

    // Both entries are listed, each carrying its per-entry state.
    let names: Vec<&str> = p.entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"alpha"), "skill listed; got {names:?}");
    assert!(names.contains(&"run-it"), "command listed; got {names:?}");

    let skill = p.entries.iter().find(|e| e.name == "alpha").unwrap();
    assert!(matches!(skill.kind, EntryKind::Skill));
    assert_eq!(skill.description, "The alpha skill.");
    assert!(skill.enabled);
    assert!(skill.searchable, "a skill is searchable by default");
    assert!(
        skill.indexed_at.is_some(),
        "an enabled entry carries indexed_at"
    );

    let cmd = p.entries.iter().find(|e| e.name == "run-it").unwrap();
    assert!(matches!(cmd.kind, EntryKind::Command));
    assert!(cmd.user_invocable, "a command is user-invocable by default");
}

#[test]
fn list_plugins_kind_filter_restricts_entries() {
    let staged = StagedWorkspace::stage(&[("alpha", SKILL)], &[("run-it", COMMAND)]);
    let harness = staged.harness();

    let out = harness
        .call_list_plugins(list_plugins::Input {
            catalog: None,
            enabled_only: true,
            kind: Some(EntryKind::Command),
        })
        .expect("list_plugins ok");

    assert_eq!(out.plugins.len(), 1);
    let p = &out.plugins[0];
    // Only the command survives the kind filter.
    assert_eq!(p.entries.len(), 1);
    assert_eq!(p.entries[0].name, "run-it");
    assert!(matches!(p.entries[0].kind, EntryKind::Command));
    // The plugin's total enabled-entry count is unaffected by the display filter.
    assert_eq!(p.enabled_entries, 2);
}

#[test]
fn list_plugins_catalog_filter_narrows_to_one_catalog() {
    let staged = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let harness = staged.harness();

    // A matching catalog returns the plugin; a non-matching one returns nothing.
    let hit = harness
        .call_list_plugins(list_plugins::Input {
            catalog: Some("acme".into()),
            enabled_only: true,
            kind: None,
        })
        .expect("ok");
    assert_eq!(hit.plugins.len(), 1);

    let miss = harness
        .call_list_plugins(list_plugins::Input {
            catalog: Some("nope".into()),
            enabled_only: true,
            kind: None,
        })
        .expect("ok");
    assert!(miss.plugins.is_empty(), "unknown catalog yields no plugins");
}

#[test]
fn list_plugins_json_shape_is_stable() {
    let staged = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let harness = staged.harness();
    let out = harness
        .call_list_plugins(list_plugins::Input {
            catalog: None,
            enabled_only: true,
            kind: None,
        })
        .expect("ok");
    let json = serde_json::to_value(&out).expect("serialise");
    // Top-level keys.
    let obj = json.as_object().expect("object");
    assert!(obj.contains_key("workspace"));
    assert!(obj.contains_key("plugins"));
    // Per-plugin + per-entry keys.
    let entry = &json["plugins"][0]["entries"][0];
    for key in [
        "name",
        "kind",
        "description",
        "enabled",
        "searchable",
        "user_invocable",
    ] {
        assert!(
            entry.get(key).is_some(),
            "entry must carry `{key}`; got: {entry}",
        );
    }
}

// ---------------------------------------------------------------------------
// list_catalogs
// ---------------------------------------------------------------------------

#[test]
fn list_catalogs_lists_the_enrolled_catalog() {
    let staged = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let harness = staged.harness();

    let out = harness
        .call_list_catalogs(list_catalogs::Input {})
        .expect("list_catalogs ok");

    assert_eq!(out.workspace, "global");
    assert_eq!(out.catalogs.len(), 1, "one enrolled catalog");
    let c = &out.catalogs[0];
    assert_eq!(c.name, "acme");
    assert!(c.url.starts_with("file://"), "url round-trips: {}", c.url);
    assert_eq!(c.ref_, "main");
}

#[test]
fn list_catalogs_json_uses_ref_key() {
    let staged = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let harness = staged.harness();
    let out = harness
        .call_list_catalogs(list_catalogs::Input {})
        .expect("ok");
    let json = serde_json::to_value(&out).expect("serialise");
    let c = &json["catalogs"][0];
    // The pinned ref serialises under the `ref` key (renamed from `ref_`).
    assert_eq!(c.get("ref").and_then(|v| v.as_str()), Some("main"));
    assert!(c.get("plugin_count").is_some(), "plugin_count present");
}

// ---------------------------------------------------------------------------
// status (with + without the doctor fold-in)
// ---------------------------------------------------------------------------

#[test]
fn status_returns_environment_snapshot_without_doctor_by_default() {
    let staged = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let harness = staged.harness();

    let out = harness
        .call_status(status::Input {
            include_doctor: false,
        })
        .expect("status ok");

    // The status report is the same shape `tome status --json` emits.
    let status = out.status.as_object().expect("status object");
    assert!(status.contains_key("tome"), "carries the tome version");
    assert!(status.contains_key("index"), "carries the index health");
    assert!(status.contains_key("entries"), "carries entry counts");
    assert_eq!(
        status.get("current_workspace").and_then(|v| v.as_str()),
        Some("global"),
    );

    // include_doctor was false → the doctor report is omitted.
    assert!(out.doctor.is_none(), "doctor must be absent by default");
    let json = serde_json::to_value(&out).expect("serialise");
    assert!(
        json.get("doctor").is_none(),
        "doctor key must be absent when include_doctor is false; got: {json}",
    );
}

#[test]
fn status_folds_in_read_only_doctor_report_when_requested() {
    let staged = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let harness = staged.harness();

    let out = harness
        .call_status(status::Input {
            include_doctor: true,
        })
        .expect("status + doctor ok");

    let doctor = out
        .doctor
        .as_ref()
        .expect("doctor report present when include_doctor is true");
    let doctor_obj = doctor.as_object().expect("doctor object");
    // Same shape `tome doctor --json` emits.
    assert!(doctor_obj.contains_key("tome_version"));
    assert!(doctor_obj.contains_key("overall"));
    assert!(
        doctor_obj.contains_key("suggested_fixes"),
        "doctor report carries suggested_fixes",
    );
    // The status snapshot is still present alongside the doctor report.
    assert!(out.status.is_object());
}

// ---------------------------------------------------------------------------
// get_skill consolidation: body vs metadata_only mode on one live entry.
// ---------------------------------------------------------------------------

#[test]
fn get_skill_body_vs_metadata_only_on_same_entry() {
    let staged = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let harness = staged.harness();

    // Body mode: content present, metadata fields absent.
    let body = harness
        .call_get_skill(get_skill::Input {
            catalog: "acme".into(),
            plugin: "plug".into(),
            name: "alpha".into(),
            kind: EntryKind::Skill,
            metadata_only: false,
            raw: false,
            include_resource_bodies: false,
        })
        .expect("body-mode get_skill ok");
    assert!(
        body.content.as_deref().unwrap().contains("Alpha body."),
        "body mode returns the rendered body",
    );
    assert!(
        body.resources_paths.is_some(),
        "body mode returns resource paths"
    );
    assert_eq!(body.substitutions_applied, Some(true));
    assert!(
        body.description.is_none(),
        "body mode omits metadata fields"
    );
    assert!(
        body.resources.is_none(),
        "body mode omits the structured enumeration"
    );

    // Metadata mode: description present, no body read.
    let meta = harness
        .call_get_skill(get_skill::Input {
            catalog: "acme".into(),
            plugin: "plug".into(),
            name: "alpha".into(),
            kind: EntryKind::Skill,
            metadata_only: true,
            raw: false,
            include_resource_bodies: false,
        })
        .expect("metadata-mode get_skill ok");
    assert_eq!(meta.description.as_deref(), Some("The alpha skill."));
    assert!(meta.content.is_none(), "metadata mode never reads the body");
    assert!(meta.substitutions_applied.is_none());
    assert!(
        meta.resources.is_some(),
        "metadata mode returns the structured resource enumeration",
    );

    // Both modes resolve the same concrete entry.
    assert_eq!(body.name, meta.name);
    assert_eq!(body.kind, meta.kind);
    assert_eq!(body.path, meta.path);
}
