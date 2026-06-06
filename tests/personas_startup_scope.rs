//! Phase 6 / US4 — FR-067 startup-scope persona toggle resolution (T4-1).
//!
//! The MCP server is NOT project-bound at runtime: it resolves
//! `expose_agents_as_personas` ONCE, at its single startup scope, via the
//! first-declarer-wins scalar walk over the (project, workspace, global)
//! settings. Project-scope layering therefore has NO effect on a running
//! server (`contracts/agent-personas.md` / FR-067).
//!
//! This pins the load-bearing claim end-to-end against ON-DISK settings
//! files (the prior coverage only exercised the pure `resolve_scalar`
//! closure with literal bools):
//!
//! 1. `resolve_expose_personas(scope, paths)` returns the STARTUP-SCOPE
//!    value (the workspace's `true`) even when an on-disk project marker
//!    declares `false` — because a workspace-startup scope carries no
//!    `project_root`, so the marker is never read.
//! 2. The persona registry is built only when the resolved scope declares
//!    `true`: with the toggle resolved to `true` the agent persona +
//!    `drop-persona` appear; resolved to `false` they do not.

mod common;

use std::fs;
use std::path::Path;

use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::index::{self, OpenOptions};
use tome::mcp::prompts::{self, PromptRegistry};
use tome::mcp::resolve_expose_personas;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

use common::{
    config_with_catalog, fabricate_models, lifecycle_paths, stub_embedder_seed, stub_reranker_seed,
    stub_summariser_seed,
};

fn open_index(paths: &tome::paths::Paths) -> rusqlite::Connection {
    index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .expect("open index db")
}

/// Lay out a single-plugin catalog shipping one agent under the `global`
/// workspace, returning the temp dir + `Paths`. Mirrors the staging in
/// `tests/personas.rs`, condensed to one plugin.
fn stage_with_agent() -> (TempDir, tome::paths::Paths) {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = tmp.path().join("catalog");
    let plugin_dir = catalog_root.join("plug");
    fs::create_dir_all(plugin_dir.join(".claude-plugin")).unwrap();
    std::fs::write(
        plugin_dir.join("tome-plugin.toml"),
        format!(
            "name = \"{}\"\nversion = \"1.0.0\"\n",
            plugin_dir.file_name().unwrap().to_string_lossy()
        ),
    )
    .unwrap();
    fs::create_dir_all(plugin_dir.join("agents")).unwrap();
    fs::write(
        catalog_root.join("tome-catalog.toml"),
        "name = \"acme\"\nversion = \"0.1.0\"\n\n[[plugins]]\nname = \"plug\"\nsource = \"./plug\"\n",
    )
    .unwrap();
    fs::write(
        plugin_dir.join(".claude-plugin").join("plugin.json"),
        r#"{"name": "plug", "version": "1.0.0"}"#,
    )
    .unwrap();
    fs::write(
        plugin_dir.join("agents").join("reviewer.md"),
        "---\nname: reviewer\ndescription: Reviewer.\n---\nReview.\n",
    )
    .unwrap();

    let config = config_with_catalog("acme", &catalog_root);
    let embedder = StubEmbedder::new();
    let scope = Scope(WorkspaceName::global());
    let deps = LifecycleDeps {
        paths: &paths,
        scope: &scope,
        config: &config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };
    let id: PluginId = "acme/plug".parse().unwrap();
    // FF1: enrolment + cache symlink must precede enable (resolve_plugin_dir
    // reads workspace_catalogs now, not the in-memory Config).
    seed_catalog_enrolment(&paths, &catalog_root, "acme");
    lifecycle::enable(&id, &deps).expect("enable plug");

    (tmp, paths)
}

fn seed_catalog_enrolment(paths: &tome::paths::Paths, catalog_root: &Path, catalog_name: &str) {
    let url = format!("file://{}", catalog_root.display());
    let conn = open_index(paths);
    tome::index::workspace_catalogs::insert(&conn, "global", catalog_name, &url, "main")
        .expect("seed workspace_catalogs");
    drop(conn);

    let cache_dir = paths.cache_dir_for(&url);
    if let Some(parent) = cache_dir.parent() {
        fs::create_dir_all(parent).expect("create catalogs parent");
    }
    if !cache_dir.exists() {
        #[cfg(unix)]
        std::os::unix::fs::symlink(catalog_root, &cache_dir).expect("symlink catalog cache");
        #[cfg(not(unix))]
        {
            fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
                fs::create_dir_all(dst)?;
                for entry in fs::read_dir(src)? {
                    let entry = entry?;
                    let to = dst.join(entry.file_name());
                    if entry.file_type()?.is_dir() {
                        copy_dir(&entry.path(), &to)?;
                    } else {
                        fs::copy(entry.path(), &to)?;
                    }
                }
                Ok(())
            }
            copy_dir(catalog_root, &cache_dir).expect("copy catalog cache");
        }
    }
}

/// Write `<root>/workspaces/global/settings.toml` declaring the persona
/// toggle.
fn write_workspace_settings(paths: &tome::paths::Paths, expose: bool) {
    let path = paths.workspace_settings_file(&WorkspaceName::global());
    fs::create_dir_all(path.parent().unwrap()).expect("create workspace settings dir");
    fs::write(
        &path,
        format!("name = \"global\"\nexpose_agents_as_personas = {expose}\n"),
    )
    .expect("write workspace settings");
}

/// Write a project marker at `<project_root>/.tome/config.toml` declaring
/// the persona toggle (and the required `workspace` binding).
fn write_project_marker(project_root: &Path, expose: bool) {
    let path = tome::paths::Paths::project_marker_config(project_root);
    fs::create_dir_all(path.parent().unwrap()).expect("create marker dir");
    fs::write(
        &path,
        format!("workspace = \"global\"\nexpose_agents_as_personas = {expose}\n"),
    )
    .expect("write project marker");
}

/// A startup scope bound to the `global` workspace with NO project root —
/// the shape the MCP server resolves under when launched without a
/// project marker. The project marker on disk must therefore be ignored.
fn workspace_startup_scope() -> ResolvedScope {
    ResolvedScope {
        scope: Scope(WorkspaceName::global()),
        source: ScopeSource::Flag,
        project_root: None,
    }
}

fn build(paths: &tome::paths::Paths, expose: bool) -> PromptRegistry {
    let conn = open_index(paths);
    PromptRegistry::build_for_workspace(&WorkspaceName::global(), paths, &conn, expose)
        .expect("build registry")
}

fn descriptor_names(registry: &PromptRegistry) -> Vec<String> {
    registry.descriptors().into_iter().map(|p| p.name).collect()
}

#[test]
fn startup_scope_ignores_project_marker_layering() {
    // Workspace says `true`; an on-disk project marker says `false`. The
    // server's startup scope carries NO project_root, so the marker is
    // never read — the resolved value is the workspace's `true`.
    let (tmp, paths) = stage_with_agent();
    write_workspace_settings(&paths, true);
    // The marker would flip it to false IF it were consulted — it must not be.
    write_project_marker(tmp.path(), false);

    let scope = workspace_startup_scope();
    let resolved = resolve_expose_personas(&scope, &paths).expect("resolve toggle");
    assert!(
        resolved,
        "startup-scope value (workspace true) wins; project-marker layering has no effect on a running server (FR-067)",
    );

    // And the registry built at that resolved value carries personas.
    let registry = build(&paths, resolved);
    let names = descriptor_names(&registry);
    assert!(
        names.contains(&"reviewer-persona".to_owned()),
        "agent persona present when the resolved toggle is true; got {names:?}",
    );
    assert!(
        names.contains(&prompts::DROP_PERSONA_NAME.to_owned()),
        "drop-persona present when the resolved toggle is true; got {names:?}",
    );
}

#[test]
fn startup_scope_false_builds_no_personas() {
    // Workspace declares `false` at the startup scope → resolved false →
    // the persona registry carries NO persona descriptors.
    let (_tmp, paths) = stage_with_agent();
    write_workspace_settings(&paths, false);

    let scope = workspace_startup_scope();
    let resolved = resolve_expose_personas(&scope, &paths).expect("resolve toggle");
    assert!(
        !resolved,
        "workspace false resolves to false at the startup scope"
    );

    let registry = build(&paths, resolved);
    let names = descriptor_names(&registry);
    assert!(
        !names.iter().any(|n| n.ends_with("-persona")),
        "no persona descriptors when the resolved toggle is false; got {names:?}",
    );
}
