//! Phase 6 / US5 — doctor extensions smoke tests (FIRST cut).
//!
//! Proves each of the five new doctor surfaces (hooks / guardrails / agents /
//! privilege-escalation / personas) populates for a bound project with
//! enabled agents shipping hooks/guardrails/privileged-fields, that they are
//! `None` outside a project (`GlobalFallback`), that the persona surface is
//! `None` when the toggle is off, that the read-only pass creates no
//! directories (FR-124), and one `--fix` happy path that re-emits agents +
//! re-renders guardrails + removes an orphan.
//!
//! The comprehensive suite (per-report drift matrices, JSON wire pins,
//! `plugin show` shape) is the next chunk; this is the compile-and-works cut.

mod common;

use std::fs;
use std::path::Path;

use tempfile::TempDir;
use tome::doctor::{self};
use tome::embedding::stub::StubEmbedder;
use tome::index::{self, OpenOptions};
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

use common::{
    HomeGuard, config_with_catalog, fabricate_models, lifecycle_paths, stub_embedder_seed,
    stub_reranker_seed, stub_summariser_seed,
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

/// Stage a single-plugin catalog shipping one PRIVILEGED agent (carries
/// `permissionMode`), a `hooks/hooks.json`, and a `hooks/GUARDRAILS.md`,
/// enabled in the `global` workspace. Returns the temp dir + `Paths`.
fn stage() -> (TempDir, tome::paths::Paths) {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = tmp.path().join("catalog");
    let plugin_dir = catalog_root.join("plug");
    fs::create_dir_all(plugin_dir.join(".claude-plugin")).unwrap();
    fs::create_dir_all(plugin_dir.join("agents")).unwrap();
    fs::create_dir_all(plugin_dir.join("hooks")).unwrap();
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
    // Privileged agent: carries `permissionMode` (FR-051 audit target).
    fs::write(
        plugin_dir.join("agents").join("reviewer.md"),
        "---\nname: reviewer\ndescription: Reviewer.\npermissionMode: ask\n---\nReview carefully.\n",
    )
    .unwrap();
    // A real hooks spec for Claude Code.
    fs::write(
        plugin_dir.join("hooks").join("hooks.json"),
        r#"{"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/guard.sh"}]}]}"#,
    )
    .unwrap();
    // A prose guardrails body.
    fs::write(
        plugin_dir.join("hooks").join("GUARDRAILS.md"),
        "Be careful with destructive operations.\n",
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
    lifecycle::enable(&id, &deps).expect("enable plug");

    seed_catalog_enrolment(&paths, &catalog_root, "acme");
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
        copy_dir(catalog_root, &cache_dir).expect("copy catalog cache");
    }
}

#[cfg(not(unix))]
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

/// Write `<root>/workspaces/global/settings.toml` declaring the persona
/// toggle.
fn write_workspace_settings(paths: &tome::paths::Paths, expose: bool) {
    let path = paths.workspace_settings_file(&WorkspaceName::global());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        format!("name = \"global\"\nexpose_agents_as_personas = {expose}\n"),
    )
    .unwrap();
}

/// Write a project marker binding to `global`.
fn write_project_marker(project_root: &Path) {
    let path = tome::paths::Paths::project_marker_config(project_root);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "workspace = \"global\"\n").unwrap();
}

/// A project-bound scope (the shape doctor resolves under inside a project).
fn project_scope(project_root: &Path) -> ResolvedScope {
    ResolvedScope {
        scope: Scope(WorkspaceName::global()),
        source: ScopeSource::ProjectMarker,
        project_root: Some(project_root.to_path_buf()),
    }
}

/// A scope WITHOUT a project root, resolved via the implicit global
/// fallback — the outside-project mode where the five surfaces are `None`.
fn global_fallback_scope() -> ResolvedScope {
    ResolvedScope {
        scope: Scope(WorkspaceName::global()),
        source: ScopeSource::GlobalFallback,
        project_root: None,
    }
}

#[test]
fn outside_project_phase6_fields_none() {
    let (_tmp, paths) = stage();
    let home = TempDir::new().unwrap();

    let scope = global_fallback_scope();
    let report = doctor::assemble_report(&scope, &paths, home.path(), false).expect("assemble");
    assert!(report.hooks.is_none(), "hooks None under GlobalFallback");
    assert!(report.guardrails.is_none(), "guardrails None");
    assert!(report.agents.is_none(), "agents None");
    assert!(
        report.privilege_escalation.is_none(),
        "privilege-escalation None"
    );
    assert!(report.personas.is_none(), "personas None");
}

#[test]
fn privilege_report_grouped_by_plugin() {
    // The privileged agent surfaces REGARDLESS of strip — the report reads
    // the source. We don't set strip; it defaults off either way, but the
    // audit reads the canonical source so the result holds in both states.
    let (tmp, paths) = stage();
    let home = TempDir::new().unwrap();
    write_project_marker(tmp.path());

    let scope = project_scope(tmp.path());
    let report = doctor::assemble_report(&scope, &paths, home.path(), false).expect("assemble");
    let pe = report
        .privilege_escalation
        .expect("privilege-escalation present in project scope");
    assert_eq!(pe.plugins.len(), 1, "one plugin with a privileged agent");
    let plug = &pe.plugins[0];
    assert_eq!(plug.catalog, "acme");
    assert_eq!(plug.plugin, "plug");
    assert_eq!(plug.agents.len(), 1);
    assert_eq!(plug.agents[0].name, "reviewer");
    assert!(
        plug.agents[0].fields.contains(&"permissionMode".to_owned()),
        "permissionMode recorded; got {:?}",
        plug.agents[0].fields
    );
}

#[test]
fn persona_report_only_when_enabled() {
    let (tmp, paths) = stage();
    let home = TempDir::new().unwrap();
    write_project_marker(tmp.path());
    let scope = project_scope(tmp.path());

    // Toggle off (no settings) → personas None.
    let off = doctor::assemble_report(&scope, &paths, home.path(), false).expect("assemble off");
    assert!(off.personas.is_none(), "personas None when toggle off");

    // Toggle on at the workspace → personas present with the reviewer slug.
    write_workspace_settings(&paths, true);
    let on = doctor::assemble_report(&scope, &paths, home.path(), false).expect("assemble on");
    let personas = on.personas.expect("personas present when on");
    assert_eq!(personas.drop_persona, "drop-persona");
    assert!(
        personas
            .personas
            .iter()
            .any(|p| p.resolved_persona_name == "reviewer-persona" && !p.clash_prefixed),
        "reviewer-persona derived; got {:?}",
        personas.personas
    );
}

#[test]
fn phase6_surface_creates_no_dirs() {
    // FR-124: a read-only doctor pass under a project scope creates no
    // `.claude/`, no harness agent dirs, no guardrails files.
    let (tmp, paths) = stage();
    let home = TempDir::new().unwrap();
    write_project_marker(tmp.path());
    let scope = project_scope(tmp.path());

    let _ = doctor::assemble_report(&scope, &paths, home.path(), false).expect("assemble");

    assert!(
        !tmp.path().join(".claude").exists(),
        ".claude must not be created by a read-only doctor pass"
    );
    assert!(
        !tmp.path().join("CLAUDE.md").exists(),
        "CLAUDE.md must not be created by a read-only doctor pass"
    );
}

#[test]
fn hooks_and_guardrails_and_agents_reports_after_sync() {
    // After a real sync the on-disk state exists; doctor's surfaces should
    // reflect it: hooks contributed, a guardrails region (suppressed on
    // Claude Code because the plugin ships real hooks), and a native agent
    // file present.
    let (tmp, paths) = stage();
    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());
    write_project_marker(tmp.path());

    // Declare the claude-code harness for the workspace so sync emits to it.
    let ws_settings = paths.workspace_settings_file(&WorkspaceName::global());
    fs::create_dir_all(ws_settings.parent().unwrap()).unwrap();
    fs::write(
        &ws_settings,
        "name = \"global\"\nharnesses = [\"claude-code\"]\n",
    )
    .unwrap();

    // Run the real sync (the production write path).
    let workspace = WorkspaceName::global();
    let deps = tome::harness::sync::SyncDeps {
        paths: &paths,
        home_root: home.path(),
        workspace_name: &workspace,
        force: false,
    };
    tome::harness::sync::sync_project(tmp.path(), &deps).expect("sync project");

    let scope = project_scope(tmp.path());
    let report = doctor::assemble_report(&scope, &paths, home.path(), false).expect("assemble");

    let hooks = report.hooks.expect("hooks present");
    assert!(
        hooks
            .plugins
            .iter()
            .any(|p| p.plugin == "plug" && p.contributed.iter().any(|c| c.event == "PreToolUse")),
        "PreToolUse hook contributed; got {hooks:?}"
    );

    let agents = report.agents.expect("agents present");
    assert!(
        agents
            .harnesses
            .iter()
            .any(|h| h.harness == "claude-code"
                && h.present.iter().any(|f| f == "plug__reviewer.md")),
        "native agent file present for claude-code; got {agents:?}"
    );

    // Guardrails: the plugin ships hooks, so Claude Code suppresses its
    // CLAUDE.md region — the region is absent on disk, so the file may not
    // appear at all. The report must at least not crash and the surface is
    // present.
    assert!(report.guardrails.is_some(), "guardrails surface present");
}

#[test]
fn fix_reemits_and_removes_orphan_agents() {
    // Sync once to emit the agent, then plant an orphan `<plugin>__*` file
    // for a plugin that is NOT enabled. `tome doctor --fix` re-runs the
    // idempotent sync which removes the orphan and re-emits the real agent.
    let (tmp, paths) = stage();
    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());
    write_project_marker(tmp.path());

    let ws_settings = paths.workspace_settings_file(&WorkspaceName::global());
    fs::create_dir_all(ws_settings.parent().unwrap()).unwrap();
    fs::write(
        &ws_settings,
        "name = \"global\"\nharnesses = [\"claude-code\"]\n",
    )
    .unwrap();

    let workspace = WorkspaceName::global();
    let deps = tome::harness::sync::SyncDeps {
        paths: &paths,
        home_root: home.path(),
        workspace_name: &workspace,
        force: false,
    };
    tome::harness::sync::sync_project(tmp.path(), &deps).expect("initial sync");

    // The claude-code agent dir holds `plug__reviewer.md` now. Plant an
    // orphan owned file for a non-enabled plugin.
    let agent_dir = tmp.path().join(".claude").join("agents");
    assert!(
        agent_dir.join("plug__reviewer.md").is_file(),
        "real agent emitted by sync"
    );
    let orphan = agent_dir.join("ghost__gone.md");
    fs::write(&orphan, "---\nname: gone\n---\nstale\n").unwrap();
    // A user-authored, non-Tome-owned file must SURVIVE the fix.
    let user_file = agent_dir.join("my-handwritten.md");
    fs::write(&user_file, "hand written\n").unwrap();

    // Run doctor --fix via the library entry (assemble + fix + re-assemble).
    let scope = project_scope(tmp.path());
    let mut report = doctor::assemble_report(&scope, &paths, home.path(), false).expect("assemble");
    let ctx = doctor::fixes::FixContext {
        paths: &paths,
        scope: &scope,
        home: home.path(),
        force: false,
    };
    let _ = doctor::fixes::apply(&mut report, &ctx);

    assert!(
        !orphan.exists(),
        "--fix must remove the orphaned <plugin>__* agent file"
    );
    assert!(
        agent_dir.join("plug__reviewer.md").is_file(),
        "--fix must keep / re-emit the real agent file"
    );
    assert!(
        user_file.exists(),
        "--fix must NEVER delete a user-authored, non-Tome-owned file"
    );
}
