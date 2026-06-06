//! Phase 6 / US5 — doctor extensions full contract matrix (T131).
//!
//! Covers each of the five new doctor surfaces (hooks / guardrails / agents /
//! privilege-escalation / personas) per `doctor-extensions-p6.md` § Tests:
//! the per-report drift matrices, the read-only invariant (FR-124), the
//! outside-project `None`-everywhere case, and the `--fix` repair classes.
//!
//! Most cases assert the read-only check functions directly against a
//! PRE-STAGED on-disk state (planted marker regions, `settings.local.json`
//! hooks, `<plugin>__*` agent files) rather than running the real
//! `sync_project` — the rules-file path triggers summarisation/index work and
//! costs ~60-70s per sync. Only the `--fix` re-emit case exercises a real sync,
//! where the end-to-end re-emit/orphan-removal IS the point.

use std::fs;
use std::path::Path;

use tempfile::TempDir;
use tome::doctor::{self};
use tome::embedding::stub::StubEmbedder;
use tome::harness::claude_code::CLAUDE_CODE;
use tome::harness::codex::CODEX;
use tome::index::{self, OpenOptions};
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

use crate::common::{
    HarnessModulesGuard, HomeGuard, config_with_catalog, fabricate_models, lifecycle_paths,
    stub_embedder_seed, stub_reranker_seed, stub_summariser_seed,
};

fn open_ro(paths: &tome::paths::Paths) -> rusqlite::Connection {
    index::open_read_only(&paths.index_db).expect("open_read_only")
}

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
    std::fs::write(
        plugin_dir.join("tome-plugin.toml"),
        format!(
            "name = \"{}\"\nversion = \"1.0.0\"\n",
            plugin_dir.file_name().unwrap().to_string_lossy()
        ),
    )
    .unwrap();
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
    // FF1: enrolment + cache symlink before enable — resolve_plugin_dir now
    // reads workspace_catalogs, not the in-memory Config.
    seed_catalog_enrolment(&paths, &catalog_root, "acme");
    lifecycle::enable(&id, &deps).expect("enable plug");
    (tmp, paths)
}

/// Insert an enabled `agent`-kind skills row for `(catalog, plugin, name)`
/// into the `global` workspace, pointing at the catalog-relative `path`.
/// The pre-staged-state pattern: no real sync, no embedding — just the rows
/// the read-only doctor surfaces consult.
fn seed_enabled_agent(
    paths: &tome::paths::Paths,
    catalog: &str,
    plugin: &str,
    name: &str,
    path: &str,
) {
    let conn = open_index(paths);
    conn.execute(
        "INSERT INTO skills
            (catalog, plugin, name, kind, description, plugin_version,
             path, content_hash, searchable, user_invocable, when_to_use, indexed_at)
         VALUES (?1, ?2, ?3, 'agent', 'desc', '0.0.0', ?4, 'h', 0, 0, NULL, '1970-01-01T00:00:00Z')",
        rusqlite::params![catalog, plugin, name, path],
    )
    .expect("insert agent row");
    let skill_id: i64 = conn
        .query_row(
            "SELECT id FROM skills WHERE catalog=?1 AND plugin=?2 AND kind='agent' AND name=?3",
            rusqlite::params![catalog, plugin, name],
            |r| r.get(0),
        )
        .expect("agent id");
    let ws_id: i64 = conn
        .query_row("SELECT id FROM workspaces WHERE name = 'global'", [], |r| {
            r.get(0)
        })
        .expect("global ws id");
    conn.execute(
        "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at) VALUES (?1, ?2, 0)",
        rusqlite::params![ws_id, skill_id],
    )
    .expect("enrol agent");
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

// ===========================================================================
// T131 — per-report drift matrices (read-only check fns, pre-staged state).
// ===========================================================================

/// Build a `<catalog>:<plugin>` guardrails marker region (matching
/// `guardrails::region_key` + the canonical START/END marker grammar).
fn guardrails_region(catalog: &str, plugin: &str, body: &str) -> String {
    format!(
        "<!-- START GUARDRAILS: {catalog}:{plugin} -->\n{body}\n<!-- END GUARDRAILS: {catalog}:{plugin} -->\n"
    )
}

#[test]
fn hooks_report_contributed_and_drift() {
    // Compute the plugin's post-rewrite hook entries directly (no sync), write
    // them into `.claude/settings.local.json`, then DROP one so the re-derived
    // entry has no structural match → it surfaces as `missing` (drift). The
    // sibling stays `contributed`.
    let (tmp, paths) = stage();
    let project_root = tmp.path();
    write_project_marker(project_root);

    // The stage fixture ships a single-event hooks.json (PreToolUse). Add a
    // second event so we have a sibling that stays contributed while we drift
    // the first. Rewrite via the production reader so the entries match what
    // `build_hooks_report` re-derives byte-for-byte.
    let conn = open_ro(&paths);
    let plugin_root = tome::index::skills::plugin_root_dir(&conn, &paths, "global", "acme", "plug")
        .expect("plugin root");
    // Append a PostToolUse event to the plugin's hooks.json so the report has
    // two events to classify.
    fs::write(
        plugin_root.join("hooks").join("hooks.json"),
        r#"{"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/guard.sh"}]}],
            "PostToolUse": [{"matcher": "Write", "hooks": [{"type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/post.sh"}]}]}"#,
    )
    .unwrap();

    let plugin_data = paths.plugin_data_dir_for("acme", "plug");
    let rewritten = tome::harness::hooks::read_rewritten_entries(&plugin_root, &plugin_data)
        .expect("read rewritten")
        .expect("hooks present");

    // Write a settings.local.json carrying ONLY the PostToolUse entries — the
    // PreToolUse entry is intentionally absent (user removed it → drift).
    let mut hooks_obj = serde_json::Map::new();
    for (event, entries) in &rewritten.events {
        if event == "PreToolUse" {
            continue; // drop → drift
        }
        hooks_obj.insert(event.clone(), serde_json::Value::Array(entries.clone()));
    }
    let settings = serde_json::json!({ "hooks": hooks_obj });
    let claude_dir = project_root.join(".claude");
    fs::create_dir_all(&claude_dir).unwrap();
    fs::write(
        claude_dir.join("settings.local.json"),
        serde_json::to_string_pretty(&settings).unwrap(),
    )
    .unwrap();

    let report = tome::doctor::checks::build_hooks_report(
        &paths,
        project_root,
        &WorkspaceName::global(),
        &conn,
    )
    .expect("hooks report");

    let plug = report
        .plugins
        .iter()
        .find(|p| p.plugin == "plug")
        .expect("plug present");
    assert!(
        plug.contributed
            .iter()
            .any(|e| e.event == "PostToolUse" && e.count == 1),
        "PostToolUse stays contributed; got {plug:?}",
    );
    assert!(
        plug.missing
            .iter()
            .any(|e| e.event == "PreToolUse" && e.count == 1),
        "the user-removed PreToolUse entry surfaces as missing (drift); got {plug:?}",
    );
}

#[test]
fn hooks_report_within_event_drift_is_entry_identity() {
    // T5-1: within-EVENT drift proves entry-identity (not event) granularity.
    // The plugin's `PreToolUse` array carries TWO distinct entries. The
    // settings.local.json carries one of them verbatim (structural match →
    // contributed) plus a HAND-EDITED copy of the other (no deep-equal →
    // missing). The SAME `PreToolUse` event must therefore appear in BOTH
    // `contributed` (count 1) and `missing` (count 1). A buggy event-
    // granularity impl (matching on the event key alone) would collapse this
    // to a single bucket and fail.
    let (tmp, paths) = stage();
    let project_root = tmp.path();
    write_project_marker(project_root);

    let conn = open_ro(&paths);
    let plugin_root = tome::index::skills::plugin_root_dir(&conn, &paths, "global", "acme", "plug")
        .expect("plugin root");
    // Two distinct entries under the single `PreToolUse` event.
    fs::write(
        plugin_root.join("hooks").join("hooks.json"),
        r#"{"PreToolUse": [
            {"matcher": "Bash", "hooks": [{"type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/a.sh"}]},
            {"matcher": "Write", "hooks": [{"type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/b.sh"}]}
        ]}"#,
    )
    .unwrap();

    let plugin_data = paths.plugin_data_dir_for("acme", "plug");
    let rewritten = tome::harness::hooks::read_rewritten_entries(&plugin_root, &plugin_data)
        .expect("read rewritten")
        .expect("hooks present");

    // Build a settings.local.json `PreToolUse` array carrying the FIRST
    // re-derived entry verbatim plus a HAND-EDITED clone of the second (so
    // the second won't structurally match what the report re-derives).
    let pre = rewritten
        .events
        .iter()
        .find(|(event, _)| event == "PreToolUse")
        .map(|(_, entries)| entries.clone())
        .expect("PreToolUse re-derived");
    assert_eq!(pre.len(), 2, "two re-derived PreToolUse entries");
    let mut edited = pre[1].clone();
    // Mutate the matcher so the entry no longer deep-equals the re-derived one.
    if let Some(obj) = edited.as_object_mut() {
        obj.insert(
            "matcher".to_owned(),
            serde_json::Value::String("Edit".to_owned()),
        );
    }
    let settings = serde_json::json!({
        "hooks": { "PreToolUse": [pre[0].clone(), edited] }
    });
    let claude_dir = project_root.join(".claude");
    fs::create_dir_all(&claude_dir).unwrap();
    fs::write(
        claude_dir.join("settings.local.json"),
        serde_json::to_string_pretty(&settings).unwrap(),
    )
    .unwrap();

    let report = tome::doctor::checks::build_hooks_report(
        &paths,
        project_root,
        &WorkspaceName::global(),
        &conn,
    )
    .expect("hooks report");

    let plug = report
        .plugins
        .iter()
        .find(|p| p.plugin == "plug")
        .expect("plug present");
    assert!(
        plug.contributed
            .iter()
            .any(|e| e.event == "PreToolUse" && e.count == 1),
        "the verbatim entry surfaces as PreToolUse contributed (count 1); got {plug:?}",
    );
    assert!(
        plug.missing
            .iter()
            .any(|e| e.event == "PreToolUse" && e.count == 1),
        "the hand-edited entry surfaces as PreToolUse missing (count 1); got {plug:?}",
    );
}

#[test]
fn agents_report_dropped_fields_populated_for_codex() {
    // T5-2: a real non-empty `dropped_fields` through Codex translation.
    // Codex drops `model` + `tools` (no OpenAI dialect carrier), so an
    // enabled agent carrying both surfaces a `DroppedFieldEntry` naming them.
    // Pre-staged state (no real sync): install ONLY codex and seed a tuned
    // agent row directly.
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(CODEX)]);

    let (tmp, paths) = stage();
    let project_root = tmp.path();
    write_project_marker(project_root);

    // Add a second agent source carrying `model:` + `tools:` to the plugin,
    // then enable it (direct DB insert — the pre-staged-state pattern).
    let plugin_agents = tmp.path().join("catalog").join("plug").join("agents");
    fs::write(
        plugin_agents.join("tuned.md"),
        "---\nname: tuned\ndescription: Tuned.\nmodel: opus\ntools:\n  - Read\n  - Grep\n---\nTuned body.\n",
    )
    .unwrap();
    seed_enabled_agent(&paths, "acme", "plug", "tuned", "agents/tuned.md");

    let conn = open_ro(&paths);
    let report = tome::doctor::checks::build_agents_report(
        &paths,
        project_root,
        &WorkspaceName::global(),
        &conn,
    )
    .expect("agents report");

    let cx = report
        .harnesses
        .iter()
        .find(|h| h.harness == "codex")
        .expect("codex harness entry");
    let dropped = cx
        .dropped_fields
        .iter()
        .find(|d| d.agent == "plug__tuned")
        .unwrap_or_else(|| panic!("plug__tuned dropped_fields recorded; got {cx:?}"));
    assert!(
        dropped.fields.contains(&"model".to_owned()),
        "codex drops `model`; got {dropped:?}",
    );
    assert!(
        dropped.fields.contains(&"tools".to_owned()),
        "codex drops `tools`; got {dropped:?}",
    );
}

#[test]
fn persona_report_clash_prefixes_same_named_agents() {
    // T5-3: two enabled agents both named `reviewer` from two different
    // plugins → the clash set marks both, and the doctor PersonaReport
    // resolves the prefixed `<plugin>-reviewer-persona` slug with
    // `clash_prefixed == true`.
    let (tmp, paths) = stage();
    // The stage fixture already enabled `acme/plug` with a `reviewer` agent.
    // Add a second plugin `plug2` also shipping a `reviewer` agent and enable
    // it (direct DB insert) so the two collide.
    let plug2_agents = tmp.path().join("catalog").join("plug2").join("agents");
    fs::create_dir_all(&plug2_agents).unwrap();
    fs::write(
        plug2_agents.join("reviewer.md"),
        "---\nname: reviewer\ndescription: Other reviewer.\n---\nReview too.\n",
    )
    .unwrap();
    seed_enabled_agent(&paths, "acme", "plug2", "reviewer", "agents/reviewer.md");

    let conn = open_ro(&paths);
    let report = tome::doctor::checks::build_persona_report(&WorkspaceName::global(), &conn)
        .expect("persona report");

    // Both `reviewer` agents clash → both prefixed.
    let clashed: Vec<_> = report
        .personas
        .iter()
        .filter(|p| p.agent_name == "reviewer")
        .collect();
    assert_eq!(
        clashed.len(),
        2,
        "two reviewer personas; got {:?}",
        report.personas
    );
    assert!(
        clashed.iter().all(|p| p.clash_prefixed),
        "both reviewer personas clash-prefixed; got {clashed:?}",
    );
    assert!(
        clashed
            .iter()
            .any(|p| p.resolved_persona_name == "plug-reviewer-persona"),
        "plug-reviewer-persona derived; got {clashed:?}",
    );
    assert!(
        clashed
            .iter()
            .any(|p| p.resolved_persona_name == "plug2-reviewer-persona"),
        "plug2-reviewer-persona derived; got {clashed:?}",
    );
}

#[test]
fn guardrails_suppressed_is_steady_state_with_no_region_on_disk() {
    // T5-4 / C5-2: a plugin shipping BOTH GUARDRAILS.md + hooks.json, enabled
    // for Claude Code, appears in the CLAUDE.md GuardrailsReport.suppressed
    // EVEN WITH NO region on disk (the steady-state correctly-synced case —
    // the real hooks supersede the prose, so the region is intentionally
    // absent). It must NOT appear in `present`.
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(CLAUDE_CODE)]);

    let (tmp, paths) = stage();
    let project_root = tmp.path();
    write_project_marker(project_root);
    // Deliberately DO NOT write any CLAUDE.md / region on disk.
    assert!(
        !project_root.join("CLAUDE.md").exists(),
        "no region on disk for the steady-state case",
    );

    let conn = open_ro(&paths);
    let report = tome::doctor::checks::build_guardrails_report(
        &paths,
        project_root,
        &WorkspaceName::global(),
        &conn,
    )
    .expect("guardrails report");

    assert_eq!(
        report.files.len(),
        1,
        "the CLAUDE.md target reports for the suppressed plugin; got {report:?}",
    );
    let file = &report.files[0];
    assert!(
        file.suppressed
            .iter()
            .any(|cp| cp.catalog == "acme" && cp.plugin == "plug"),
        "acme:plug suppressed in steady state (ships GUARDRAILS.md + hooks.json); got {file:?}",
    );
    assert!(
        file.present.is_empty(),
        "no region is present on disk; got {file:?}",
    );
}

#[test]
fn guardrails_report_present_orphan_suppressed() {
    // Plant a CLAUDE.md with two regions: the enabled `acme:plug` (which ships
    // real hooks → suppressed on Claude Code) and an orphan `gone:ghost`
    // (plugin not enabled). Install ONLY claude-code so the target is
    // deterministic.
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(CLAUDE_CODE)]);

    let (tmp, paths) = stage();
    let project_root = tmp.path();
    write_project_marker(project_root);

    let body = format!(
        "{}{}",
        guardrails_region("acme", "plug", "Be careful."),
        guardrails_region("gone", "ghost", "Stale region."),
    );
    fs::write(project_root.join("CLAUDE.md"), body).unwrap();

    let conn = open_ro(&paths);
    let report = tome::doctor::checks::build_guardrails_report(
        &paths,
        project_root,
        &WorkspaceName::global(),
        &conn,
    )
    .expect("guardrails report");

    assert_eq!(report.files.len(), 1, "one target file (CLAUDE.md)");
    let file = &report.files[0];
    // Both regions are present on disk.
    assert!(
        file.present
            .iter()
            .any(|cp| cp.catalog == "acme" && cp.plugin == "plug"),
        "acme:plug present; got {file:?}",
    );
    assert!(
        file.present
            .iter()
            .any(|cp| cp.catalog == "gone" && cp.plugin == "ghost"),
        "gone:ghost present; got {file:?}",
    );
    // The not-enabled plugin's region is orphaned.
    assert!(
        file.orphaned
            .iter()
            .any(|cp| cp.catalog == "gone" && cp.plugin == "ghost"),
        "gone:ghost orphaned; got {file:?}",
    );
    assert!(
        !file.orphaned.iter().any(|cp| cp.plugin == "plug"),
        "the enabled plugin is NOT orphaned; got {file:?}",
    );
    // The enabled plugin ships real hooks → its CLAUDE.md region is suppressed.
    assert!(
        file.suppressed
            .iter()
            .any(|cp| cp.catalog == "acme" && cp.plugin == "plug"),
        "acme:plug suppressed (ships real hooks, FR-013); got {file:?}",
    );
}

#[test]
fn agents_report_present_orphan_dropped() {
    // Plant `<plugin>__*` agent files directly in the claude-code agent dir:
    // the enabled `plug__reviewer.md` (present) and an orphan
    // `ghost__gone.md` (plugin not enabled → orphaned). The privileged
    // `permissionMode` field on the enabled agent is dropped during
    // claude-code translation? No — Claude Code passes those through. To
    // exercise `dropped_fields` we rely on the source agent's frontmatter
    // being re-translated; Claude Code keeps all three, so dropped_fields is
    // empty here. The present/orphaned split is the load-bearing assertion.
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(CLAUDE_CODE)]);

    let (tmp, paths) = stage();
    let project_root = tmp.path();
    write_project_marker(project_root);

    let agent_dir = project_root.join(".claude").join("agents");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::write(
        agent_dir.join("plug__reviewer.md"),
        "---\nname: reviewer\n---\nReview carefully.\n",
    )
    .unwrap();
    fs::write(
        agent_dir.join("ghost__gone.md"),
        "---\nname: gone\n---\nstale\n",
    )
    .unwrap();
    // A user-authored, non-`<plugin>__*` file must NOT appear in either list.
    fs::write(agent_dir.join("my-handwritten.md"), "hand written\n").unwrap();

    let conn = open_ro(&paths);
    let report = tome::doctor::checks::build_agents_report(
        &paths,
        project_root,
        &WorkspaceName::global(),
        &conn,
    )
    .expect("agents report");

    let cc = report
        .harnesses
        .iter()
        .find(|h| h.harness == "claude-code")
        .expect("claude-code harness entry");

    assert!(
        cc.present.iter().any(|f| f == "plug__reviewer.md"),
        "enabled agent file present; got {cc:?}",
    );
    assert!(
        cc.present.iter().any(|f| f == "ghost__gone.md"),
        "orphan owned file is present on disk; got {cc:?}",
    );
    assert!(
        !cc.present.iter().any(|f| f == "my-handwritten.md"),
        "a non-`<plugin>__*` user file is NOT a Tome-owned present file; got {cc:?}",
    );
    assert!(
        cc.orphaned.iter().any(|f| f == "ghost__gone.md"),
        "the not-enabled plugin's file is orphaned; got {cc:?}",
    );
    assert!(
        !cc.orphaned.iter().any(|f| f == "plug__reviewer.md"),
        "the enabled plugin's file is NOT orphaned; got {cc:?}",
    );
}

#[test]
fn fix_rerenders_stale_guardrails() {
    // A real-sync `--fix` case: plant a STALE guardrails region for the
    // enabled plugin in a harness that does NOT suppress (we install a stub
    // GuardrailsOnly harness via codex so the region is rendered, not
    // suppressed). After `--fix` the region body matches the plugin's current
    // GUARDRAILS.md. We use codex (GuardrailsOnly, in-file AGENTS.md region).
    let (tmp, paths) = stage();
    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());
    let project_root = tmp.path();
    write_project_marker(project_root);

    // The stage fixture's catalog cache is a symlink to a non-git dir, so
    // doctor would classify it Missing/NotARepo and `--fix` would try to
    // RE-CLONE it — destroying the cache the guardrails re-render reads from.
    // Plant a `.git/` in the symlinked catalog so doctor classifies it Ok and
    // the catalog repair never fires. (The guardrails re-render IS the
    // subject under test; the catalog repair is incidental noise.)
    fs::create_dir_all(tmp.path().join("catalog").join(".git")).unwrap();

    // Declare codex so sync emits a guardrails region (codex is
    // GuardrailsOnly — no hook suppression — so the region renders).
    let ws_settings = paths.workspace_settings_file(&WorkspaceName::global());
    fs::create_dir_all(ws_settings.parent().unwrap()).unwrap();
    fs::write(&ws_settings, "name = \"global\"\nharnesses = [\"codex\"]\n").unwrap();

    let workspace = WorkspaceName::global();
    let deps = tome::harness::sync::SyncDeps {
        paths: &paths,
        home_root: home.path(),
        workspace_name: &workspace,
        force: false,
    };
    tome::harness::sync::sync_project(project_root, &deps).expect("initial sync");

    // Codex's guardrails target is its rules-file (AGENTS.md). Find the file
    // that now carries the region and corrupt the body between the markers.
    let agents_md = project_root.join("AGENTS.md");
    assert!(agents_md.is_file(), "codex guardrails landed in AGENTS.md");
    let before = fs::read_to_string(&agents_md).unwrap();
    assert!(
        before.contains("START GUARDRAILS: acme:plug"),
        "the plugin's region was rendered; got:\n{before}",
    );
    // Stomp the body inside the region while leaving the markers intact.
    let stale = before.replace("Be careful with destructive operations.", "STALE HAND EDIT");
    fs::write(&agents_md, &stale).unwrap();

    // doctor --fix re-runs the idempotent sync → re-renders the region body.
    let scope = project_scope(project_root);
    let mut report = doctor::assemble_report(&scope, &paths, home.path(), false).expect("assemble");
    let ctx = doctor::fixes::FixContext {
        paths: &paths,
        scope: &scope,
        home: home.path(),
        force: false,
    };
    let _ = doctor::fixes::apply(&mut report, &ctx);

    let after = fs::read_to_string(&agents_md).unwrap();
    assert!(
        after.contains("Be careful with destructive operations."),
        "--fix re-renders the stale guardrails body; got:\n{after}",
    );
    assert!(
        !after.contains("STALE HAND EDIT"),
        "the stale body is overwritten between the markers; got:\n{after}",
    );
}

#[test]
fn fix_never_removes_unowned_hook() {
    // A user-edited hook entry in `.claude/settings.local.json` does NOT
    // structurally match any re-derived plugin entry, so `--fix` (which only
    // appends/removes structural matches) MUST leave it in place (NFR-003).
    let (tmp, paths) = stage();
    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());
    let project_root = tmp.path();
    write_project_marker(project_root);
    // Keep the catalog cache healthy so the plugin's real hooks ARE derived
    // and merged — the user hook surviving ALONGSIDE them is the point.
    fs::create_dir_all(tmp.path().join("catalog").join(".git")).unwrap();

    let ws_settings = paths.workspace_settings_file(&WorkspaceName::global());
    fs::create_dir_all(ws_settings.parent().unwrap()).unwrap();
    fs::write(
        &ws_settings,
        "name = \"global\"\nharnesses = [\"claude-code\"]\n",
    )
    .unwrap();

    // Seed a USER-OWNED hook entry that no plugin would derive.
    let claude_dir = project_root.join(".claude");
    fs::create_dir_all(&claude_dir).unwrap();
    let user_hook = serde_json::json!({
        "hooks": {
            "PreToolUse": [
                {"matcher": "Read", "hooks": [{"type": "command", "command": "/my/own/script.sh"}]}
            ]
        }
    });
    let settings_path = claude_dir.join("settings.local.json");
    fs::write(
        &settings_path,
        serde_json::to_string_pretty(&user_hook).unwrap(),
    )
    .unwrap();

    let scope = project_scope(project_root);
    let mut report = doctor::assemble_report(&scope, &paths, home.path(), false).expect("assemble");
    let ctx = doctor::fixes::FixContext {
        paths: &paths,
        scope: &scope,
        home: home.path(),
        force: false,
    };
    let _ = doctor::fixes::apply(&mut report, &ctx);

    let after = fs::read_to_string(&settings_path).expect("settings.local.json survives");
    assert!(
        after.contains("/my/own/script.sh"),
        "--fix must NEVER remove a hook it cannot prove it owns; got:\n{after}",
    );
}

#[test]
fn fix_never_deletes_user_content() {
    // `--fix` must not delete rules-file text outside Tome markers. Plant a
    // CLAUDE.md with hand-written prose plus a Tome guardrails region; after
    // `--fix` the hand-written prose survives verbatim.
    let (tmp, paths) = stage();
    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());
    let project_root = tmp.path();
    write_project_marker(project_root);
    fs::create_dir_all(tmp.path().join("catalog").join(".git")).unwrap();

    let ws_settings = paths.workspace_settings_file(&WorkspaceName::global());
    fs::create_dir_all(ws_settings.parent().unwrap()).unwrap();
    fs::write(&ws_settings, "name = \"global\"\nharnesses = [\"codex\"]\n").unwrap();

    // Hand-written content the user owns, outside any Tome marker.
    let agents_md = project_root.join("AGENTS.md");
    let user_prose = "# My project notes\n\nThese are MY notes, hands off.\n";
    fs::write(&agents_md, user_prose).unwrap();

    let scope = project_scope(project_root);
    let mut report = doctor::assemble_report(&scope, &paths, home.path(), false).expect("assemble");
    let ctx = doctor::fixes::FixContext {
        paths: &paths,
        scope: &scope,
        home: home.path(),
        force: false,
    };
    let _ = doctor::fixes::apply(&mut report, &ctx);

    let after = fs::read_to_string(&agents_md).expect("AGENTS.md survives");
    assert!(
        after.contains("These are MY notes, hands off."),
        "--fix must preserve user-authored text outside Tome markers; got:\n{after}",
    );
}
