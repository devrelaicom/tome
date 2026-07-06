//! Issue #431 — doctor hook-drift checks for the dispatcher harnesses.
//!
//! Drives the REAL `sync_project` over the `cursor` module (the
//! `hook_translate_cursor` fixture shape), then probes the doctor surface
//! through `build_hook_translation_report`: the per-harness `state` must
//! classify ok / drift / missing / stale_manifest against the SAME expected
//! set the sync writer computes, and `doctor --fix` (the coalesced harness
//! re-sync) must heal a seeded drift. Claude Code's own hooks surface
//! (`build_hooks_report` over `.claude/settings.local.json`) is untouched by
//! this feature — its behaviour is pinned by the existing `doctor_p6` tests.

use std::path::{Path, PathBuf};

use crate::common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::harness::sync::{self, SyncDeps};
use tome::settings::resolver::{EffectiveHarness, EffectiveHarnessList};
use tome::workspace::WorkspaceName;

struct Fixture {
    home: TempDir,
    paths: tome::paths::Paths,
    project: PathBuf,
    workspace: WorkspaceName,
}

impl Fixture {
    fn deps(&self) -> SyncDeps<'_> {
        SyncDeps {
            paths: &self.paths,
            home_root: self.home.path(),
            workspace_name: &self.workspace,
            force: false,
            only_harness: None,
            dry_run: false,
        }
    }
}

fn build() -> Fixture {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).expect("create tome root");
    seed_workspace(&paths, "test-workspace");
    let workspace = WorkspaceName::parse("test-workspace").expect("parse workspace");

    let project = env.home_path().join("project");
    std::fs::create_dir_all(&project).expect("create project");
    let marker_dir = project.join(".tome");
    std::fs::create_dir_all(&marker_dir).expect("create marker dir");
    std::fs::write(
        marker_dir.join("config.toml"),
        "workspace = \"test-workspace\"\nharnesses = [\"cursor\"]\n",
    )
    .expect("write marker");
    std::fs::write(marker_dir.join("RULES.md"), "# rules\n").expect("write rules");

    Fixture {
        home: env.home,
        paths,
        project,
        workspace,
    }
}

/// Seed a plugin shipping a `PreToolUse` Bash command hook + enrol/enable it
/// (the `hook_translate_cursor` fixture shape).
fn seed_plugin(fx: &Fixture) {
    let url = String::from("https://example.test/plugin-a.git");
    let hooks_dir = fx.paths.cache_dir_for(&url).join("plugin-a").join("hooks");
    std::fs::create_dir_all(&hooks_dir).expect("create hooks source dir");
    std::fs::write(
        hooks_dir.join("hooks.json"),
        r#"{ "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "/opt/guard.sh check" } ] } ] }"#,
    )
    .expect("write source hooks.json");

    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "test-workspace", "cat", &url, "main")
        .expect("enrol catalog");
    conn.execute(
        "INSERT INTO skills
            (catalog, plugin, name, kind, description, plugin_version,
             path, content_hash, searchable, user_invocable, when_to_use, indexed_at)
         VALUES ('cat','plugin-a','demo','skill','d','0.0.0','skills/demo/SKILL.md','h',1,0,NULL,'1970-01-01T00:00:00Z')",
        [],
    )
    .expect("insert skill row");
    let skill_id: i64 = conn
        .query_row(
            "SELECT id FROM skills WHERE catalog='cat' AND plugin='plugin-a'",
            [],
            |r| r.get(0),
        )
        .expect("skill id");
    let ws_id: i64 = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name='test-workspace'",
            [],
            |r| r.get(0),
        )
        .expect("ws id");
    conn.execute(
        "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at) VALUES (?1, ?2, 0)",
        rusqlite::params![ws_id, skill_id],
    )
    .expect("enrol skill");
}

fn effective_cursor() -> EffectiveHarnessList {
    EffectiveHarnessList {
        harnesses: vec![EffectiveHarness {
            name: "cursor".to_owned(),
            source_chain: vec!["project".to_owned()],
        }],
        excluded: vec![],
    }
}

/// Probe the doctor surface and return the cursor row's `(state, missing)`.
fn probe(fx: &Fixture) -> (String, Vec<String>) {
    let cfg = tome::config::Config::default();
    let report = tome::doctor::checks::build_hook_translation_report(
        &fx.paths,
        &fx.workspace,
        &cfg,
        Some(&effective_cursor()),
        fx.home.path(),
        Some(&fx.project),
    );
    let row = report
        .per_harness
        .iter()
        .find(|h| h.harness == "cursor")
        .expect("cursor row present");
    (
        row.state.clone().expect("probed in project context"),
        row.missing_events.clone(),
    )
}

fn hook_file(fx: &Fixture) -> PathBuf {
    fx.project.join(".cursor/hooks.json")
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).expect("read")).expect("parse")
}

#[test]
fn seeded_cursor_drift_states_detected_and_fix_heals() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::cursor::Cursor)]);

    let fx = build();
    seed_plugin(&fx);

    // ---- Missing: expected entries, but sync never ran (no hook file). ----
    let (state, missing) = probe(&fx);
    assert_eq!(state, "missing", "pre-sync: nothing registered");
    assert_eq!(missing, vec!["PreToolUse".to_owned()]);

    // ---- Ok after a real sync. ----
    sync::sync_project(&fx.project, &fx.deps()).expect("sync");
    let (state, missing) = probe(&fx);
    assert_eq!(state, "ok", "freshly synced surface is ok");
    assert!(missing.is_empty(), "{missing:?}");

    // ---- Drift: the Tome run-hook entry is edited out of the hook file. ----
    let mut doc = read_json(&hook_file(&fx));
    doc["hooks"]["preToolUse"] = serde_json::json!([]);
    std::fs::write(
        hook_file(&fx),
        serde_json::to_string_pretty(&doc).unwrap() + "\n",
    )
    .expect("corrupt hook file");
    let (state, missing) = probe(&fx);
    assert_eq!(state, "drift", "unregistered used event is drift");
    assert_eq!(missing, vec!["PreToolUse".to_owned()]);

    // ---- Stale manifest: hook file healthy, manifest corrupted. ----
    sync::sync_project(&fx.project, &fx.deps()).expect("re-sync");
    let manifest_path = fx.paths.hooks_manifest(&fx.workspace, "cursor");
    std::fs::write(&manifest_path, "{}\n").expect("corrupt manifest");
    let (state, _) = probe(&fx);
    assert_eq!(
        state, "stale_manifest",
        "manifest mismatch is stale_manifest"
    );

    // ---- `doctor --fix` heals: the drift finding is auto-fixable and the ----
    // coalesced harness re-sync re-registers the entry + rewrites the manifest.
    doc["hooks"]["preToolUse"] = serde_json::json!([]);
    std::fs::write(
        hook_file(&fx),
        serde_json::to_string_pretty(&doc).unwrap() + "\n",
    )
    .expect("re-corrupt hook file");
    crate::common::fabricate_all_registry_models(&fx.paths);

    let scope = tome::workspace::ResolvedScope {
        scope: tome::workspace::Scope(fx.workspace.clone()),
        source: tome::workspace::ScopeSource::ProjectMarker,
        project_root: Some(fx.project.clone()),
        overridden_project_marker: None,
    };
    let mut report =
        tome::doctor::assemble_report(&scope, &fx.paths, fx.home.path(), false).expect("assemble");

    // The finding: an auto-fixable `harness-hooks:cursor` fix + Degraded.
    let fix = report
        .suggested_fixes
        .iter()
        .find(|f| f.subsystem.to_wire_string() == "harness-hooks:cursor")
        .expect("hook-drift fix present");
    assert!(fix.auto_fixable, "hook drift is safely auto-fixable");
    assert_eq!(fix.command, "tome sync");
    // The escalation is at-least-Degraded (this fixture's stub-seeded DB also
    // reports embedder drift → Unhealthy wins the monotone max; the exact
    // Degraded step is unit-tested beside `push_hook_drift_fixes`).
    assert_ne!(
        report.overall,
        tome::doctor::DoctorClassification::Ok,
        "hook drift must escalate overall",
    );

    // Apply: the coalesced harness sync re-registers the entry. Retain ONLY
    // the fix under test — this fixture's hand-made catalog cache is not a
    // real git clone, so the unrelated auto catalog re-clone fix would tear
    // down the plugin source mid-test (a fixture artefact, not a product
    // behaviour). The retained fix exercises exactly the #431 routing:
    // HarnessHooks → the coalesced `repair_harness_sync_with` dispatch.
    report
        .suggested_fixes
        .retain(|f| f.subsystem.to_wire_string() == "harness-hooks:cursor");
    let ctx = tome::doctor::fixes::FixContext {
        paths: &fx.paths,
        scope: &scope,
        home: fx.home.path(),
        force: false,
    };
    let _ = tome::doctor::fixes::apply(&mut report, &ctx);

    let healed = read_json(&hook_file(&fx));
    let arr = healed["hooks"]["preToolUse"]
        .as_array()
        .expect("preToolUse array restored");
    assert!(
        !arr.is_empty(),
        "the run-hook entry must be re-registered by --fix",
    );
    // The refreshed report reflects the healed state.
    let ht = report.hook_translation.as_ref().expect("refreshed");
    let row = ht
        .per_harness
        .iter()
        .find(|h| h.harness == "cursor")
        .expect("cursor row");
    assert_eq!(row.state.as_deref(), Some("ok"), "post-fix state is ok");

    // Claude Code untouched: the dispatcher probe never creates or edits
    // `.claude/settings.local.json`.
    assert!(
        !fx.project.join(".claude/settings.local.json").exists(),
        "the dispatcher drift check must not touch Claude Code's settings",
    );
}

/// Outside a project context the probe cannot run (the native hook file is
/// per-project): rows carry `state: None` and no drift finding is pushed.
#[test]
fn no_project_context_leaves_state_unprobed() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::cursor::Cursor)]);

    let fx = build();
    seed_plugin(&fx);
    let cfg = tome::config::Config::default();
    let report = tome::doctor::checks::build_hook_translation_report(
        &fx.paths,
        &fx.workspace,
        &cfg,
        Some(&effective_cursor()),
        fx.home.path(),
        None,
    );
    let row = report
        .per_harness
        .iter()
        .find(|h| h.harness == "cursor")
        .expect("cursor row present");
    assert!(row.state.is_none(), "no project root → no probe");
    assert!(row.missing_events.is_empty());
}
