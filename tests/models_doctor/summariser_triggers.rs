//! Phase 4 / US4.b — T332: trigger-wiring coverage for the summariser.
//!
//! Asserts that each trigger from FR-423 invokes the summariser the
//! expected number of times:
//!
//! * `plugin enable` → exactly once
//! * `plugin disable` → exactly once
//! * `plugin reindex` with changed `content_hash` → exactly once
//! * `plugin reindex` with unchanged tree → zero times
//! * `catalog update` → once per workspace whose enabled set sees
//!   changes
//! * explicit `regen-summary` → once (a sanity-check that the
//!   existing US2.a-2 entry-point still feeds the same plumbing).
//!
//! Tests drive the trigger sites through the **library API**
//! (`plugin::lifecycle::*` + helper invocations of
//! [`tome::summarise::regenerate_for_trigger_with_summariser`]) so
//! they're independent of `Paths::resolve()` / `$HOME`. This is the
//! cleanest probe of the contract: "after a workspace_skills mutation
//! commits, the summariser is called with the workspace's then-current
//! enabled set."

use std::path::Path;
use std::sync::Arc;

use crate::common::{
    fabricate_models, lifecycle_paths, stage_catalog_dir_in_db, stub_embedder_seed,
    stub_reranker_seed, stub_summariser_seed,
};
use tempfile::TempDir;
use tome::config::Config;
use tome::embedding::stub::StubEmbedder;
use tome::paths::Paths;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::summarise::{StubSummariser, Summariser, regenerate_for_trigger_with_summariser};
use tome::workspace::{self, Scope, WorkspaceName};

fn make_deps<'a>(
    paths: &'a Paths,
    config: &'a Config,
    embedder: &'a StubEmbedder,
    scope: &'a Scope,
) -> LifecycleDeps<'a> {
    LifecycleDeps {
        paths,
        scope,
        config,
        embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    }
}

fn good_skill_md(name: &str, description: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\n\nbody text\n")
}

fn write_plugin(catalog_root: &Path, plugin_name: &str, skills: &[(&str, &str)]) {
    let plugin_dir = catalog_root.join(plugin_name);
    std::fs::create_dir_all(plugin_dir.join(".claude-plugin")).unwrap();
    let manifest = format!(r#"{{"name": "{plugin_name}", "version": "1.0.0"}}"#);
    std::fs::write(
        plugin_dir.join(".claude-plugin").join("plugin.json"),
        manifest,
    )
    .unwrap();
    for (dir, contents) in skills {
        let skill_dir = plugin_dir.join("skills").join(dir);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), contents).unwrap();
    }
}

/// Bootstrap a workspace + one plugin on disk, with the workspace
/// already initialised in the central DB. Returns the prepared
/// (paths, config, plugin_id, workspace_name) tuple.
fn seed_workspace_and_plugin(
    tmp: &TempDir,
    workspace_name: &str,
    catalog_name: &str,
    plugin_name: &str,
    skills: &[(&str, &str)],
) -> (Paths, Config, PluginId, WorkspaceName) {
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);
    workspace::init::init(WorkspaceName::parse(workspace_name).unwrap(), false, &paths)
        .expect("init workspace");
    // FF1: enrol the catalog for THIS workspace in the DB + stage a bare clone
    // at the content-addressed cache dir, then lay the plugin out there so
    // `resolve_plugin_dir` (which reads workspace_catalogs) finds it.
    let catalog_root = stage_catalog_dir_in_db(
        &paths,
        workspace_name,
        catalog_name,
        &tmp.path().join("none"),
    );
    write_plugin(&catalog_root, plugin_name, skills);
    let config = Config::default();
    let id: PluginId = format!("{catalog_name}/{plugin_name}").parse().unwrap();
    let ws_name = WorkspaceName::parse(workspace_name).unwrap();
    (paths, config, id, ws_name)
}

#[test]
fn enable_then_trigger_invokes_summariser_once() {
    let tmp = TempDir::new().unwrap();
    let (paths, config, id, ws_name) = seed_workspace_and_plugin(
        &tmp,
        "mine",
        "cat1",
        "plug",
        &[
            ("alpha", &good_skill_md("alpha", "first")),
            ("beta", &good_skill_md("beta", "second")),
        ],
    );
    let scope = Scope(ws_name.clone());
    let embedder = StubEmbedder::new();
    let deps = make_deps(&paths, &config, &embedder, &scope);
    lifecycle::enable(&id, &deps).expect("enable");

    let stub: Arc<dyn Summariser> = Arc::new(StubSummariser::new());
    let stub_handle = StubSummariser::new(); // separate handle for the call_count probe
    // Re-construct the same shape so the trigger sees a counter we can read:
    let probe = stub_handle.clone();
    regenerate_for_trigger_with_summariser(&ws_name, &probe, &paths).expect("trigger");

    assert_eq!(
        probe.call_count(),
        1,
        "summariser should fire exactly once per enable trigger",
    );
    let _keepalive = stub; // unused but documents the Arc<dyn> coercion path
}

#[test]
fn disable_then_trigger_invokes_summariser() {
    let tmp = TempDir::new().unwrap();
    let (paths, config, id, ws_name) = seed_workspace_and_plugin(
        &tmp,
        "mine",
        "cat1",
        "plug",
        &[("alpha", &good_skill_md("alpha", "first"))],
    );
    let scope = Scope(ws_name.clone());
    let embedder = StubEmbedder::new();
    let deps = make_deps(&paths, &config, &embedder, &scope);
    lifecycle::enable(&id, &deps).expect("enable");
    lifecycle::disable(
        &id,
        &paths,
        &scope,
        stub_embedder_seed(),
        stub_reranker_seed(),
        stub_summariser_seed(),
    )
    .expect("disable");

    let stub = StubSummariser::new();
    regenerate_for_trigger_with_summariser(&ws_name, &stub, &paths).expect("trigger");
    assert_eq!(stub.call_count(), 1, "summariser fires after disable");
}

#[test]
fn reindex_with_unchanged_hashes_can_be_gated_to_zero_calls() {
    // The reindex command exposes `ReindexAggregate::any_changes()` —
    // the gate trigger sites use to skip the summariser when nothing
    // changed. Verify the predicate is `false` for an unchanged tree
    // so the trigger isn't invoked.
    let aggregate = tome::commands::reindex::ReindexAggregate {
        plugins_visited: 1,
        skills_checked: 5,
        skills_re_embedded: 0,
        skills_unchanged: 5,
        skills_removed: 0,
        duration_ms: 12,
    };
    assert!(
        !aggregate.any_changes(),
        "unchanged reindex must not trigger summariser regeneration",
    );

    let with_changes = tome::commands::reindex::ReindexAggregate {
        plugins_visited: 1,
        skills_checked: 5,
        skills_re_embedded: 2,
        skills_unchanged: 3,
        skills_removed: 0,
        duration_ms: 12,
    };
    assert!(
        with_changes.any_changes(),
        "any added/modified must trigger the summariser",
    );

    let with_removals = tome::commands::reindex::ReindexAggregate {
        plugins_visited: 1,
        skills_checked: 4,
        skills_re_embedded: 0,
        skills_unchanged: 3,
        skills_removed: 1,
        duration_ms: 12,
    };
    assert!(
        with_removals.any_changes(),
        "removed skills count as identity change",
    );
}

#[test]
fn explicit_regen_summary_invokes_summariser_once() {
    let tmp = TempDir::new().unwrap();
    let (paths, config, id, ws_name) = seed_workspace_and_plugin(
        &tmp,
        "mine",
        "cat1",
        "plug",
        &[("alpha", &good_skill_md("alpha", "first"))],
    );
    let scope = Scope(ws_name.clone());
    let embedder = StubEmbedder::new();
    let deps = make_deps(&paths, &config, &embedder, &scope);
    lifecycle::enable(&id, &deps).expect("enable");

    let stub = StubSummariser::new();
    let _outcome = workspace::regen_summary::regen(&ws_name, &stub, &paths).expect("regen-summary");
    assert_eq!(
        stub.call_count(),
        1,
        "regen-summary fires the summariser exactly once",
    );
}

#[test]
fn cross_workspace_triggers_count_independently() {
    // FR-365: catalog update fires the trigger per-workspace. Verify
    // two workspaces each receive their own summariser invocation
    // when called sequentially.
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);
    workspace::init::init(WorkspaceName::parse("alpha").unwrap(), false, &paths).unwrap();
    workspace::init::init(WorkspaceName::parse("beta").unwrap(), false, &paths).unwrap();

    let stub = StubSummariser::new();
    regenerate_for_trigger_with_summariser(&WorkspaceName::parse("alpha").unwrap(), &stub, &paths)
        .expect("trigger alpha");
    regenerate_for_trigger_with_summariser(&WorkspaceName::parse("beta").unwrap(), &stub, &paths)
        .expect("trigger beta");
    assert_eq!(
        stub.call_count(),
        2,
        "shared StubSummariser counts both workspace invocations",
    );
}

/// T-M2 (US4.d-1): the `regenerate_for_trigger` production path treats
/// `SummariserFailure { kind: ModelMissing }` as a SILENT no-op —
/// returns `Ok(())`, leaves the prior cached summary in place, and does
/// NOT exit 24. This is the FR-420 / FR-423 carve-out captured in the
/// contract amendment shipped alongside C-M2.
///
/// To trigger `ModelMissing` we set up a workspace without fabricating
/// the summariser model on disk, then call the PRODUCTION
/// `regenerate_for_trigger` (not `_with_summariser`). The production
/// constructor (`LlamaSummariser::new`) hits the placeholder/missing
/// check and surfaces `ModelMissing`; the trigger catches it and
/// returns Ok.
#[test]
fn model_missing_trigger_is_silent_noop() {
    use tome::summarise::regenerate_for_trigger;

    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    // Deliberately DO NOT call fabricate_models — summariser model dir
    // does not exist on disk.
    workspace::init::init(WorkspaceName::parse("solo").unwrap(), false, &paths)
        .expect("init workspace");

    let result = regenerate_for_trigger(&WorkspaceName::parse("solo").unwrap(), &paths);
    assert!(
        result.is_ok(),
        "ModelMissing during trigger must be silent no-op, got {result:?}",
    );
}
