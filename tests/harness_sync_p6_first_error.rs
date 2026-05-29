//! Phase 6 polish (TEST-1) — multi-sink `first_error` PRECEDENCE + forward
//! progress.
//!
//! `src/harness/sync.rs` surfaces failures from the three Phase 6 sinks in a
//! FIXED precedence after running every sink for forward progress:
//!
//! ```text
//!   first_clash (19)  ->  hooks (43)  ->  guardrails (46)  ->  agents (45)
//! ```
//!
//! (See `sync_project`'s tail: the `if let Some(..) = ..first_error` checks run
//! hooks, then guardrails, then agents — confirmed in `sync.rs`.) The per-sink
//! suites each prove ONE sink failing in isolation; none drives more than one
//! sink failing in the SAME sync. A reorder of those three checks would pass
//! every existing test yet silently change which exit code a user sees when two
//! sinks fail at once. These tests pin the order:
//!
//! * Test A: hooks + guardrails + agents all fail in one sync alongside a fully
//!   healthy plugin. The surfaced code is 43 (hooks wins), and the healthy
//!   plugin's hook entry + guardrails region + agent file ALL land (forward
//!   progress crossed all three sinks despite the failures).
//! * Test B: the same seeding minus the hooks-malformed plugin. The surfaced
//!   code is now 46 (guardrails is next in the fixed order) — proving the
//!   precedence is real, not a coincidence of which sink happened to run first.
//!
//! ## Independent failures on independent plugins
//!
//! Each failing component lives on its OWN `(catalog, plugin)` so the three
//! sink passes each enumerate + reject one distinct plugin:
//!   1. `plugin-hooks`   ships an unparsable `hooks/hooks.json`   -> exit 43.
//!   2. `plugin-guard`   ships a marker-shaped `GUARDRAILS.md`    -> exit 46.
//!   3. `plugin-agent`   ships malformed agent YAML frontmatter   -> exit 45.
//!   4. `plugin-healthy` ships valid versions of all three.
//!
//! A single `StubHarness` (RealJson hooks + settings + native agents + the
//! default non-suppressing in-file guardrails region) drives all three sinks.

mod common;

use std::path::PathBuf;
use std::sync::Mutex;

use common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::harness::sync::{self, SyncDeps};
use tome::harness::{AgentFormat, HooksStrategy, StubHarness};
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

/// Install the single all-three-sinks stub harness.
fn install_stub() -> HarnessModulesGuard {
    HarnessModulesGuard::install(vec![Box::new(
        StubHarness::default()
            .with_hooks_strategy(HooksStrategy::RealJson)
            .with_hook_settings()
            .with_native_agents(AgentFormat::MarkdownYaml),
    )])
}

// --- per-component source seeders (manifest-less plugin-root cache) ---------

fn plugin_url(plugin: &str) -> String {
    format!("https://example.test/{plugin}.git")
}

fn seed_hooks_source(paths: &tome::paths::Paths, plugin: &str, body: &str) {
    let dir = paths
        .cache_dir_for(&plugin_url(plugin))
        .join(plugin)
        .join("hooks");
    std::fs::create_dir_all(&dir).expect("create hooks source dir");
    std::fs::write(dir.join("hooks.json"), body).expect("write source hooks.json");
}

fn seed_guardrails_source(paths: &tome::paths::Paths, plugin: &str, body: &str) {
    let dir = paths
        .cache_dir_for(&plugin_url(plugin))
        .join(plugin)
        .join("hooks");
    std::fs::create_dir_all(&dir).expect("create hooks source dir");
    std::fs::write(dir.join("GUARDRAILS.md"), body).expect("write GUARDRAILS.md");
}

fn seed_agent_source(paths: &tome::paths::Paths, plugin: &str, name: &str, body: &str) {
    let dir = paths
        .cache_dir_for(&plugin_url(plugin))
        .join(plugin)
        .join("agents");
    std::fs::create_dir_all(&dir).expect("create agent source dir");
    std::fs::write(dir.join(format!("{name}.md")), body).expect("write source agent");
}

fn enrol_catalog(paths: &tome::paths::Paths, ws: &str, catalog: &str, plugin: &str) {
    let conn = rusqlite::Connection::open(&paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, ws, catalog, &plugin_url(plugin), "main")
        .expect("enrol catalog");
}

/// Insert an enabled `skill`-kind row so the plugin is enumerated for the hooks
/// and guardrails passes (which key on DISTINCT catalog/plugin of any enabled
/// row). Used for plugins that do not need to participate in the agent path.
fn insert_enabled_skill_row(paths: &tome::paths::Paths, ws: &str, catalog: &str, plugin: &str) {
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
    enrol_skill_id(&conn, ws, catalog, plugin, "skill");
}

/// Insert an enabled `agent`-kind row so the plugin participates in the agent
/// path AND (via the DISTINCT catalog/plugin enumeration) the hooks + guardrails
/// passes.
fn insert_enabled_agent_row(
    paths: &tome::paths::Paths,
    ws: &str,
    catalog: &str,
    plugin: &str,
    name: &str,
) {
    let conn = rusqlite::Connection::open(&paths.index_db).expect("open rw");
    conn.execute(
        "INSERT INTO skills
            (catalog, plugin, name, kind, description, plugin_version,
             path, content_hash, searchable, user_invocable, when_to_use, indexed_at)
         VALUES (?1, ?2, ?3, 'agent', 'desc', '0.0.0', ?4, 'h', 0, 0, NULL, '1970-01-01T00:00:00Z')",
        rusqlite::params![catalog, plugin, name, format!("agents/{name}.md")],
    )
    .expect("insert agent row");
    enrol_skill_id(&conn, ws, catalog, plugin, "agent");
}

/// Enrol the just-inserted `(catalog, plugin, kind)` row into `workspace_skills`.
fn enrol_skill_id(conn: &rusqlite::Connection, ws: &str, catalog: &str, plugin: &str, kind: &str) {
    let skill_id: i64 = conn
        .query_row(
            "SELECT id FROM skills WHERE catalog=?1 AND plugin=?2 AND kind=?3",
            rusqlite::params![catalog, plugin, kind],
            |r| r.get(0),
        )
        .expect("skill id");
    let ws_id: i64 = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name = ?1",
            rusqlite::params![ws],
            |r| r.get(0),
        )
        .expect("ws id");
    conn.execute(
        "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at) VALUES (?1, ?2, 0)",
        rusqlite::params![ws_id, skill_id],
    )
    .expect("enrol skill");
}

/// Seed the fully-healthy plugin shipping valid versions of all three
/// components, enrolled + enabled via an agent row (so it appears in every
/// sink's enumeration).
fn seed_healthy_plugin(fx: &Fixture) {
    seed_hooks_source(
        &fx.paths,
        "plugin-healthy",
        r#"{ "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/guard.sh" } ] } ] }"#,
    );
    seed_guardrails_source(&fx.paths, "plugin-healthy", "Be careful with deletes.\n");
    seed_agent_source(
        &fx.paths,
        "plugin-healthy",
        "reviewer",
        "---\nname: reviewer\ndescription: Reviews code.\n---\nYou review code.\n",
    );
    enrol_catalog(&fx.paths, "test-ws", "cat-healthy", "plugin-healthy");
    insert_enabled_agent_row(
        &fx.paths,
        "test-ws",
        "cat-healthy",
        "plugin-healthy",
        "reviewer",
    );
}

/// Seed the guardrails-poisoned plugin: a body whose line matches the managed
/// END marker is rejected by the B-1 fail-closed validator (exit 46). Mirrors
/// `tests/guardrails_marker_injection.rs::stray_end_body_is_rejected_*`.
fn seed_guardrails_poison_plugin(fx: &Fixture) {
    seed_guardrails_source(
        &fx.paths,
        "plugin-guard",
        "do the thing\n<!-- END GUARDRAILS: c:p -->\n",
    );
    enrol_catalog(&fx.paths, "test-ws", "cat-guard", "plugin-guard");
    insert_enabled_skill_row(&fx.paths, "test-ws", "cat-guard", "plugin-guard");
}

/// Seed the agent-corrupt plugin: a well-formed agent row is enabled, but the
/// on-disk source has malformed frontmatter (no closing delimiter) so
/// `prepare_agent` fails (exit 45). Mirrors
/// `harness_sync_stub.rs::agent_forward_progress_one_corrupt_one_good`.
fn seed_agent_corrupt_plugin(fx: &Fixture) {
    seed_agent_source(
        &fx.paths,
        "plugin-agent",
        "builder",
        "---\nname: builder\nno closing delimiter here\n",
    );
    enrol_catalog(&fx.paths, "test-ws", "cat-agent", "plugin-agent");
    insert_enabled_agent_row(&fx.paths, "test-ws", "cat-agent", "plugin-agent", "builder");
}

/// Assert the healthy plugin's three sinks ALL landed on disk (forward progress
/// crossed every sink despite the sibling failures).
fn assert_healthy_plugin_landed(fx: &Fixture) {
    // Hooks: the healthy plugin's rewritten command is in settings.local.json.
    let hooks_path = fx.project.join(".stub/settings.local.json");
    assert!(
        hooks_path.is_file(),
        "the healthy plugin's hooks must merge despite the failing siblings"
    );
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&hooks_path).unwrap()).unwrap();
    let cmd = doc["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .expect("healthy plugin's rewritten command present");
    let plugin_root = fx
        .paths
        .cache_dir_for(&plugin_url("plugin-healthy"))
        .join("plugin-healthy");
    assert!(
        cmd.starts_with(&*plugin_root.to_string_lossy()),
        "the healthy plugin's hook landed (rewritten): {cmd}"
    );

    // Guardrails: the healthy plugin's region rendered in the rules-file target.
    let rendered = std::fs::read_to_string(fx.project.join("STUB_RULES.md"))
        .expect("guardrails target exists");
    assert!(
        rendered.contains("<!-- START GUARDRAILS: cat-healthy:plugin-healthy -->"),
        "the healthy plugin's guardrails region rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("Be careful with deletes."),
        "the healthy plugin's guardrails body rendered:\n{rendered}"
    );

    // Agents: the healthy plugin's agent file landed.
    assert!(
        fx.project
            .join(".stub/agents/plugin-healthy__reviewer.md")
            .is_file(),
        "the healthy plugin's agent file landed despite the failing siblings"
    );
}

#[test]
fn hooks_wins_precedence_over_guardrails_and_agents() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = install_stub();

    let fx = Fixture::build("test-ws", "\"stub\"");

    // (1) hooks-malformed plugin: unparsable JSON -> exit 43 class.
    seed_hooks_source(&fx.paths, "plugin-hooks", "{ this is not valid json");
    enrol_catalog(&fx.paths, "test-ws", "cat-hooks", "plugin-hooks");
    insert_enabled_skill_row(&fx.paths, "test-ws", "cat-hooks", "plugin-hooks");

    // (2) guardrails-poisoned plugin -> exit 46 class.
    seed_guardrails_poison_plugin(&fx);
    // (3) agent-corrupt plugin -> exit 45 class.
    seed_agent_corrupt_plugin(&fx);
    // (4) fully healthy plugin -> all three sinks valid.
    seed_healthy_plugin(&fx);

    // All three sinks fail in one sync; the fixed order surfaces HOOKS (43).
    let err = sync::sync_project(&fx.project, &fx.deps())
        .expect_err("three sink failures must surface an error");
    assert_eq!(
        err.exit_code(),
        43,
        "hooks (43) wins precedence over guardrails (46) and agents (45); got {err:?}"
    );

    // Forward progress crossed all three sinks for the healthy plugin.
    assert_healthy_plugin_landed(&fx);
}

#[test]
fn guardrails_wins_when_hooks_failure_absent() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = install_stub();

    let fx = Fixture::build("test-ws", "\"stub\"");

    // Identical seeding to Test A MINUS the hooks-malformed plugin. With no
    // hooks failure, the next sink in the fixed order — guardrails (46) — wins.
    seed_guardrails_poison_plugin(&fx);
    seed_agent_corrupt_plugin(&fx);
    seed_healthy_plugin(&fx);

    let err = sync::sync_project(&fx.project, &fx.deps())
        .expect_err("guardrails + agent failures must surface an error");
    assert_eq!(
        err.exit_code(),
        46,
        "with no hooks failure, guardrails (46) wins over agents (45); got {err:?}"
    );

    // Forward progress still landed the healthy plugin's three sinks.
    assert_healthy_plugin_landed(&fx);
}
