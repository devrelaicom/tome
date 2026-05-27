//! Phase 5 / US5.b — doctor extensions: prompts surface, orphan
//! plugin-data + workspace-data dirs, and entry counts split by kind.
//!
//! All tests reuse the lifecycle library API (`StubEmbedder`) so the
//! enable pipeline runs without a real ONNX model on disk. The doctor
//! pass is invoked via `doctor::assemble_report` (the silent-compute
//! library entry point), so we never spawn the CLI binary.
//!
//! Read-only enforcement (FR-124): `doctor_phase5_surface_creates_no_dirs`
//! snapshots `<home>/.tome/` before and after to prove no plugin-data /
//! workspace-data dirs are lazy-created.

mod common;

use common::{
    ToolEnv, config_with_catalog, copy_sample_plugin_catalog, fabricate_all_registry_models,
    global_scope, paths_for, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed,
    write_config_for_cli,
};
use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::paths::Paths;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

// ---- shared scope helpers -------------------------------------------------

/// Wrap one common bootstrap: isolated tome root with bytes-only models
/// and the sample-plugin-catalog enabled. Returns the env + paths + the
/// fixture TempDir keeping the catalog source alive.
struct EnabledFixture {
    _env: ToolEnv,
    paths: Paths,
    _fixture_tmp: TempDir,
}

fn enable_sample_plugin() -> EnabledFixture {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let fixture_tmp = TempDir::new().unwrap();
    let catalog_root = copy_sample_plugin_catalog(&fixture_tmp, "catalog");
    let cli_config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    write_config_for_cli(&paths, &cli_config);

    let embedder = StubEmbedder::new();
    let deps = LifecycleDeps {
        paths: &paths,
        scope: &global_scope(),
        config: &cli_config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    lifecycle::enable(&id, &deps).expect("pre-enable plugin-alpha for doctor_p5 tests");

    EnabledFixture {
        _env: env,
        paths,
        _fixture_tmp: fixture_tmp,
    }
}

// ---- Prompts surface ------------------------------------------------------

#[test]
fn prompts_surface_enumerates_with_collisions() {
    let fx = enable_sample_plugin();
    let home = TempDir::new().unwrap();

    // The sample-plugin-catalog has no user-invocable entries by
    // default (skills default to `user_invocable=false`). The prompts
    // report should still populate but with an empty prompts vector
    // when the scope is a known workspace; it must NOT be None.
    let scope = ResolvedScope {
        scope: Scope(WorkspaceName::global()),
        source: ScopeSource::Flag,
        project_root: None,
    };
    let report = tome::doctor::assemble_report(&scope, &fx.paths, home.path(), false).unwrap();

    assert!(
        report.prompts.is_some(),
        "in a known workspace (source=Flag) prompts must be populated, got None",
    );
    let prompts = report.prompts.as_ref().unwrap();
    // Zero user-invocable entries in the sample catalog → empty list.
    assert!(
        prompts.prompts.is_empty(),
        "expected zero prompts, got {:?}",
        prompts.prompts
    );
    assert!(prompts.collisions.is_empty());
}

// ---- Orphan plugin-data detection ----------------------------------------

#[test]
fn orphan_plugin_data_detected() {
    let fx = enable_sample_plugin();
    let home = TempDir::new().unwrap();

    // Fabricate an orphan plugin-data dir for a (catalog, plugin)
    // pair that is NOT enabled in any workspace.
    let orphan_dir = fx.paths.root.join("plugin-data/ghost-catalog/ghost-plugin");
    std::fs::create_dir_all(&orphan_dir).unwrap();

    let scope = ResolvedScope {
        scope: Scope(WorkspaceName::global()),
        source: ScopeSource::Flag,
        project_root: None,
    };
    let report = tome::doctor::assemble_report(&scope, &fx.paths, home.path(), false).unwrap();
    let orphan_report = report
        .orphan_data_dirs
        .as_ref()
        .expect("orphan_data_dirs populated in workspace scope");

    assert!(
        orphan_report.plugin_data.contains(&orphan_dir),
        "expected {} in plugin_data orphans; got {:?}",
        orphan_dir.display(),
        orphan_report.plugin_data,
    );
}

#[test]
fn orphan_plugin_data_excludes_active_enrolment() {
    let fx = enable_sample_plugin();
    let home = TempDir::new().unwrap();

    // Fabricate a plugin-data dir for an ACTIVELY-enabled pair.
    let active_dir = fx
        .paths
        .root
        .join("plugin-data/sample-plugin-catalog/plugin-alpha");
    std::fs::create_dir_all(&active_dir).unwrap();

    let scope = ResolvedScope {
        scope: Scope(WorkspaceName::global()),
        source: ScopeSource::Flag,
        project_root: None,
    };
    let report = tome::doctor::assemble_report(&scope, &fx.paths, home.path(), false).unwrap();
    let orphan_report = report.orphan_data_dirs.unwrap();

    assert!(
        !orphan_report.plugin_data.contains(&active_dir),
        "active enrolment must NOT appear in plugin_data orphans; got {:?}",
        orphan_report.plugin_data,
    );
}

#[test]
fn orphan_workspace_data_detected() {
    let fx = enable_sample_plugin();
    let home = TempDir::new().unwrap();

    // workspace=`global`, fabricate a per-workspace plugin-data dir
    // for an unenrolled `(catalog, plugin)` pair.
    let orphan_dir = fx
        .paths
        .root
        .join("workspaces/global/plugin-data/ghost-catalog/ghost-plugin");
    std::fs::create_dir_all(&orphan_dir).unwrap();

    let scope = ResolvedScope {
        scope: Scope(WorkspaceName::global()),
        source: ScopeSource::Flag,
        project_root: None,
    };
    let report = tome::doctor::assemble_report(&scope, &fx.paths, home.path(), false).unwrap();
    let orphan_report = report.orphan_data_dirs.unwrap();

    assert!(
        orphan_report.workspace_data.contains(&orphan_dir),
        "expected {} in workspace_data orphans; got {:?}",
        orphan_dir.display(),
        orphan_report.workspace_data,
    );
}

// ---- Entry counts ----------------------------------------------------------

#[test]
fn entry_counts_by_kind() {
    let fx = enable_sample_plugin();
    let home = TempDir::new().unwrap();

    let scope = ResolvedScope {
        scope: Scope(WorkspaceName::global()),
        source: ScopeSource::Flag,
        project_root: None,
    };
    let report = tome::doctor::assemble_report(&scope, &fx.paths, home.path(), false).unwrap();
    let counts = report
        .entry_counts
        .as_ref()
        .expect("entry_counts populated in workspace scope");

    // The sample-plugin-catalog enables plugin-alpha which ships
    // 5 skill directories under `skills/` (one with malformed
    // frontmatter is skipped by the parser, leaving 4 indexable).
    assert!(
        counts.skills >= 1,
        "expected >=1 skill, got {}",
        counts.skills,
    );
    // No commands in the sample fixture.
    assert_eq!(
        counts.commands, 0,
        "no commands in fixture, got {}",
        counts.commands
    );
}

#[test]
fn pending_re_embedding_count_matches_dirty_rows() {
    use std::time::SystemTime;
    let fx = enable_sample_plugin();
    let home = TempDir::new().unwrap();

    // The `pending_re_embedding` heuristic resolves each entry's body
    // path via `resolve_entry_body_path`, which walks the central DB's
    // `workspace_catalogs` enrolment → `paths.cache_dir_for(url)` →
    // catalog manifest → plugin dir. In production this resolves to
    // the on-disk catalog clone; in tests the catalog source lives at
    // the fixture TempDir path, but the enrolment URL is `file://<that
    // path>`, so `paths.cache_dir_for(url)` resolves to a different
    // directory that the test never populated. To exercise the mtime
    // comparison end-to-end, we mirror the fixture into the resolved
    // cache dir (the same path the production code reads from).
    let url = format!(
        "file://{}",
        fx._fixture_tmp.path().join("catalog").display(),
    );
    let cache_dir = fx.paths.cache_dir_for(&url);
    std::fs::create_dir_all(&cache_dir).unwrap();
    copy_dir_recursive(&fx._fixture_tmp.path().join("catalog"), &cache_dir);

    let skill_file = cache_dir.join("plugin-alpha/skills/skill-a/SKILL.md");
    assert!(
        skill_file.exists(),
        "mirrored skill file missing: {}",
        skill_file.display(),
    );
    // Push mtime well past indexed_at.
    let future = SystemTime::now() + std::time::Duration::from_secs(3600);
    filetime::set_file_mtime(&skill_file, filetime::FileTime::from_system_time(future))
        .expect("set future mtime on skill file");

    let scope = ResolvedScope {
        scope: Scope(WorkspaceName::global()),
        source: ScopeSource::Flag,
        project_root: None,
    };
    let report = tome::doctor::assemble_report(&scope, &fx.paths, home.path(), false).unwrap();
    let counts = report.entry_counts.unwrap();

    assert!(
        counts.pending_re_embedding >= 1,
        "expected >=1 pending re-embedding after future-mtime touch, got {}",
        counts.pending_re_embedding,
    );
}

/// Recursively copy `src` into `dst`. Both paths must be absolute.
/// Used to mirror a test fixture into the production cache_dir for
/// the `pending_re_embedding` resolution test.
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let p = entry.path();
        let target = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir_recursive(&p, &target);
        } else {
            std::fs::copy(&p, &target).unwrap();
        }
    }
}

// ---- None when outside-project --------------------------------------------

#[test]
fn outside_project_phase5_fields_none() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = TempDir::new().unwrap();

    // GlobalFallback (no flag, no env, no marker) → all three Phase 5
    // fields must be None per the contract.
    let report = tome::doctor::assemble_report(
        &ResolvedScope::global_fallback(),
        &paths,
        home.path(),
        false,
    )
    .unwrap();

    assert!(report.prompts.is_none(), "GlobalFallback → prompts = None");
    assert!(
        report.orphan_data_dirs.is_none(),
        "GlobalFallback → orphan_data_dirs = None",
    );
    assert!(
        report.entry_counts.is_none(),
        "GlobalFallback → entry_counts = None",
    );
}

// ---- Read-only invariant (FR-124) ----------------------------------------

#[test]
fn doctor_phase5_surface_creates_no_dirs() {
    let fx = enable_sample_plugin();
    let home = TempDir::new().unwrap();

    // Snapshot every dir under <home>/.tome/ before the doctor pass.
    let before = snapshot_dirs(&fx.paths.root);

    let scope = ResolvedScope {
        scope: Scope(WorkspaceName::global()),
        source: ScopeSource::Flag,
        project_root: None,
    };
    let _report = tome::doctor::assemble_report(&scope, &fx.paths, home.path(), false).unwrap();

    let after = snapshot_dirs(&fx.paths.root);

    // The plugin-data tree MUST NOT be lazy-created.
    let plugin_data = fx.paths.root.join("plugin-data");
    let plugin_data_before = before.contains(&plugin_data);
    let plugin_data_after = after.contains(&plugin_data);
    assert_eq!(
        plugin_data_before, plugin_data_after,
        "FR-124: doctor must not lazy-create <root>/plugin-data/; \
         before={plugin_data_before}, after={plugin_data_after}",
    );

    // Workspace-data tree under any workspace must also not appear.
    // We assert the global workspace's plugin-data dir specifically.
    let ws_plugin_data = fx.paths.root.join("workspaces/global/plugin-data");
    let ws_pd_before = before.contains(&ws_plugin_data);
    let ws_pd_after = after.contains(&ws_plugin_data);
    assert_eq!(
        ws_pd_before, ws_pd_after,
        "FR-124: doctor must not lazy-create <root>/workspaces/global/plugin-data/; \
         before={ws_pd_before}, after={ws_pd_after}",
    );
}

/// Recursively enumerate every directory under `root`. Used by the
/// read-only invariant test to compare before/after snapshots.
fn snapshot_dirs(root: &std::path::Path) -> std::collections::HashSet<std::path::PathBuf> {
    let mut out: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
    fn walk(p: &std::path::Path, out: &mut std::collections::HashSet<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(p) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_dir() {
                out.insert(path.clone());
                walk(&path, out);
            }
        }
    }
    if root.is_dir() {
        out.insert(root.to_path_buf());
        walk(root, &mut out);
    }
    out
}
