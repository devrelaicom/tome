//! Phase 6 / US3 — Claude Code guardrails suppression (T091, FR-013/016).
//!
//! - A plugin shipping BOTH `GUARDRAILS.md` and `hooks/hooks.json` has its
//!   region suppressed on `CLAUDE.md` (real hooks supersede prose) but
//!   present on the shared `AGENTS.md`.
//! - Both transitions handled in one sync: a plugin that BEGINS shipping
//!   `hooks.json` has its `CLAUDE.md` region removed while hooks merge; one
//!   that CEASES has its hooks removed while the region re-renders.

mod common;

use std::path::PathBuf;
use std::sync::Mutex;

use common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::harness::sync::{self, SyncDeps};
use tome::workspace::WorkspaceName;

static OVERRIDE_MUTEX: Mutex<()> = Mutex::new(());

struct Fixture {
    _home: TempDir,
    paths: tome::paths::Paths,
    project: PathBuf,
    workspace: WorkspaceName,
}

impl Fixture {
    fn build(workspace_name: &str, harnesses: &str) -> Self {
        let env = ToolEnv::new();
        let paths = paths_for(&env);
        std::fs::create_dir_all(&paths.root).expect("create tome root");
        seed_workspace(&paths, workspace_name);
        let workspace = WorkspaceName::parse(workspace_name).expect("parse workspace");

        let project = env.home_path().join("project");
        std::fs::create_dir_all(&project).expect("create project");
        let marker_dir = project.join(".tome");
        std::fs::create_dir_all(&marker_dir).expect("create marker dir");
        std::fs::write(
            marker_dir.join("config.toml"),
            format!("workspace = \"{workspace_name}\"\nharnesses = [{harnesses}]\n"),
        )
        .expect("write marker");
        std::fs::write(marker_dir.join("RULES.md"), "# rules\n").expect("write rules");

        Fixture {
            _home: env.home,
            paths,
            project,
            workspace,
        }
    }

    fn deps(&self) -> SyncDeps<'_> {
        SyncDeps {
            paths: &self.paths,
            home_root: self._home.path(),
            workspace_name: &self.workspace,
            force: false,
        }
    }
}

/// The plugin-root cache dir for `plugin` under a stable example URL.
fn plugin_url(plugin: &str) -> String {
    format!("https://example.test/{plugin}.git")
}

fn write_guardrails(paths: &tome::paths::Paths, plugin: &str, body: &str) {
    let cache = paths.cache_dir_for(&plugin_url(plugin));
    let dir = cache.join(plugin).join("hooks");
    std::fs::create_dir_all(&dir).expect("create hooks dir");
    std::fs::write(dir.join("GUARDRAILS.md"), body).expect("write GUARDRAILS.md");
}

fn write_hooks_json(paths: &tome::paths::Paths, plugin: &str, body: &str) {
    let cache = paths.cache_dir_for(&plugin_url(plugin));
    let dir = cache.join(plugin).join("hooks");
    std::fs::create_dir_all(&dir).expect("create hooks dir");
    std::fs::write(dir.join("hooks.json"), body).expect("write hooks.json");
}

fn remove_hooks_json(paths: &tome::paths::Paths, plugin: &str) {
    let cache = paths.cache_dir_for(&plugin_url(plugin));
    let p = cache.join(plugin).join("hooks").join("hooks.json");
    std::fs::remove_file(p).expect("remove hooks.json");
}

fn insert_enabled_skill_row(
    paths: &tome::paths::Paths,
    workspace: &str,
    catalog: &str,
    plugin: &str,
) {
    let conn = rusqlite::Connection::open(&paths.index_db).expect("open rw");
    conn.execute(
        "INSERT INTO skills
            (catalog, plugin, name, kind, description, plugin_version,
             path, content_hash, searchable, user_invocable, when_to_use, indexed_at)
         VALUES (?1, ?2, 'demo', 'skill', 'd', '0.0.0',
                 'skills/demo/SKILL.md', 'h', 1, 0, NULL, '1970-01-01T00:00:00Z')",
        rusqlite::params![catalog, plugin],
    )
    .expect("insert skill row");
    let skill_id: i64 = conn
        .query_row(
            "SELECT id FROM skills WHERE catalog=?1 AND plugin=?2 AND kind='skill'",
            rusqlite::params![catalog, plugin],
            |r| r.get(0),
        )
        .expect("skill id");
    let ws_id: i64 = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name = ?1",
            rusqlite::params![workspace],
            |r| r.get(0),
        )
        .expect("ws id");
    conn.execute(
        "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at) VALUES (?1, ?2, 0)",
        rusqlite::params![ws_id, skill_id],
    )
    .expect("enrol skill");
}

fn enrol_catalog(paths: &tome::paths::Paths, ws: &str, catalog: &str, plugin: &str) {
    let conn = rusqlite::Connection::open(&paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, ws, catalog, &plugin_url(plugin), "main")
        .expect("enrol catalog");
}

const HOOKS_JSON: &str = r#"{ "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/guard.sh" } ] } ] }"#;

#[test]
fn both_shipping_plugin_suppressed_on_claude_md_present_on_agents() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    // claude-code (suppress) + codex (shared AGENTS.md, no suppress).
    let _guard = HarnessModulesGuard::install(vec![
        Box::new(tome::harness::claude_code::CLAUDE_CODE),
        Box::new(tome::harness::codex::CODEX),
    ]);

    let fx = Fixture::build("test-ws", "\"claude-code\", \"codex\"");
    write_guardrails(&fx.paths, "plugin-a", "be careful\n");
    write_hooks_json(&fx.paths, "plugin-a", HOOKS_JSON);
    enrol_catalog(&fx.paths, "test-ws", "cat", "plugin-a");
    insert_enabled_skill_row(&fx.paths, "test-ws", "cat", "plugin-a");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    let marker = "<!-- START GUARDRAILS: cat:plugin-a -->";

    // CLAUDE.md: region SUPPRESSED (real hooks supersede). The file may exist
    // for the rules block, but must not carry the guardrails region.
    let claude = std::fs::read_to_string(fx.project.join("CLAUDE.md")).unwrap_or_default();
    assert!(
        !claude.contains(marker),
        "CLAUDE.md region must be suppressed when the plugin ships hooks.json:\n{claude}"
    );

    // Real hooks merged into settings.local.json.
    assert!(
        fx.project.join(".claude/settings.local.json").is_file(),
        "hooks must merge into settings.local.json"
    );

    // AGENTS.md: region PRESENT (codex has no suppression).
    let agents = std::fs::read_to_string(fx.project.join("AGENTS.md")).unwrap();
    assert!(
        agents.contains(marker),
        "AGENTS.md region must be present (no suppression for codex):\n{agents}"
    );
}

#[test]
fn both_transitions_in_one_sync() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard =
        HarnessModulesGuard::install(vec![Box::new(tome::harness::claude_code::CLAUDE_CODE)]);

    let fx = Fixture::build("test-ws", "\"claude-code\"");

    // plugin-starts: ships GUARDRAILS.md only at first; gains hooks.json later.
    write_guardrails(&fx.paths, "plugin-starts", "starts rules\n");
    enrol_catalog(&fx.paths, "test-ws", "cat-s", "plugin-starts");
    insert_enabled_skill_row(&fx.paths, "test-ws", "cat-s", "plugin-starts");

    // plugin-stops: ships BOTH at first (region suppressed); loses hooks.json
    // later (region must re-render).
    write_guardrails(&fx.paths, "plugin-stops", "stops rules\n");
    write_hooks_json(&fx.paths, "plugin-stops", HOOKS_JSON);
    enrol_catalog(&fx.paths, "test-ws", "cat-t", "plugin-stops");
    insert_enabled_skill_row(&fx.paths, "test-ws", "cat-t", "plugin-stops");

    let m_starts = "<!-- START GUARDRAILS: cat-s:plugin-starts -->";
    let m_stops = "<!-- START GUARDRAILS: cat-t:plugin-stops -->";

    // ----- sync 1: starts rendered, stops suppressed -----
    sync::sync_project(&fx.project, &fx.deps()).expect("sync 1");
    let claude = std::fs::read_to_string(fx.project.join("CLAUDE.md")).unwrap();
    assert!(
        claude.contains(m_starts),
        "starts rendered initially:\n{claude}"
    );
    assert!(
        !claude.contains(m_stops),
        "stops suppressed initially (ships hooks.json):\n{claude}"
    );

    // ----- transition both plugins, then sync 2 -----
    // plugin-starts BEGINS shipping hooks.json → region removed, hooks merge.
    write_hooks_json(&fx.paths, "plugin-starts", HOOKS_JSON);
    // plugin-stops CEASES shipping hooks.json → hooks removed, region renders.
    remove_hooks_json(&fx.paths, "plugin-stops");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync 2");
    let claude = std::fs::read_to_string(fx.project.join("CLAUDE.md")).unwrap();
    assert!(
        !claude.contains(m_starts),
        "plugin-starts region removed once it ships hooks.json:\n{claude}"
    );
    assert!(
        claude.contains(m_stops),
        "plugin-stops region re-rendered once it stops shipping hooks.json:\n{claude}"
    );

    // plugin-starts's hooks now merged into settings.local.json.
    let doc: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(fx.project.join(".claude/settings.local.json")).unwrap(),
    )
    .unwrap();
    assert!(
        doc["hooks"]["PreToolUse"].is_array(),
        "plugin-starts hooks merged after it begins shipping hooks.json: {doc}"
    );
}
