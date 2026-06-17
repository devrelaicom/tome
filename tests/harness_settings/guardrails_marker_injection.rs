//! Phase 6 / US3 — B-1 fail-closed marker validation of `GUARDRAILS.md` bodies.
//!
//! A guardrails body is copied verbatim between Tome's managed markers and
//! re-parsed on every sync. A body line that itself looks like a managed
//! marker (a guardrails START/END line, or a `tome:begin/end` block marker)
//! would let a plugin escape its region, wedge the file, or corrupt the rules
//! block. `read_guardrails_source` rejects such bodies (exit 46, naming the
//! source). These tests prove three crafted bodies are each rejected while a
//! legitimate sibling plugin's region STILL renders, and that a re-sync stays
//! convergent (no wedge).

use std::path::PathBuf;

use crate::common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::harness::sync::{self, SyncDeps};
use tome::workspace::WorkspaceName;

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
            only_harness: None,
        }
    }
}

fn seed_guardrails_source(paths: &tome::paths::Paths, plugin: &str, body: &str) -> String {
    let url = format!("https://example.test/{plugin}.git");
    let cache = paths.cache_dir_for(&url);
    let hooks_dir = cache.join(plugin).join("hooks");
    std::fs::create_dir_all(&hooks_dir).expect("create hooks dir");
    std::fs::write(hooks_dir.join("GUARDRAILS.md"), body).expect("write GUARDRAILS.md");
    url
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

fn enrol(paths: &tome::paths::Paths, ws: &str, catalog: &str, url: &str) {
    let conn = rusqlite::Connection::open(&paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, ws, catalog, url, "main")
        .expect("enrol catalog");
}

/// Run the shared scenario: one POISONED plugin (rejected) alongside one CLEAN
/// sibling (must still render). Asserts the sibling region lands on AGENTS.md,
/// the poisoned region never does, and a re-sync stays convergent (no wedge).
fn poisoned_plus_sibling(poison_plugin: &str, poison_body: &str) {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::codex::CODEX)]);

    let fx = Fixture::build("test-ws", "\"codex\"");

    // The poisoned plugin. "cat-bad" sorts before "cat-good" so its region,
    // were it ever rendered, would be the first in the file.
    let url_bad = seed_guardrails_source(&fx.paths, poison_plugin, poison_body);
    enrol(&fx.paths, "test-ws", "cat-bad", &url_bad);
    insert_enabled_skill_row(&fx.paths, "test-ws", "cat-bad", poison_plugin);

    // The clean sibling — its region MUST still render despite the bad sibling.
    let url_good = seed_guardrails_source(&fx.paths, "plugin-good", "Be careful with deletes.\n");
    enrol(&fx.paths, "test-ws", "cat-good", &url_good);
    insert_enabled_skill_row(&fx.paths, "test-ws", "cat-good", "plugin-good");

    // Forward progress: the sync surfaces the poisoned plugin's exit-46 error
    // but does NOT abort the clean sibling's reconciliation.
    let err = sync::sync_project(&fx.project, &fx.deps())
        .expect_err("a marker-poisoned body must surface an error");
    assert_eq!(
        err.exit_code(),
        46,
        "poisoned guardrails body → exit 46; got {err:?}"
    );
    // The error names the offending source file.
    let msg = err.to_string();
    assert!(
        msg.contains(poison_plugin),
        "exit-46 message must name the offending source ({poison_plugin}):\n{msg}"
    );

    let good_marker = "<!-- START GUARDRAILS: cat-good:plugin-good -->";
    let bad_marker = format!("<!-- START GUARDRAILS: cat-bad:{poison_plugin} -->");

    let agents = std::fs::read_to_string(fx.project.join("AGENTS.md")).unwrap();
    assert!(
        agents.contains(good_marker),
        "the clean sibling's region must still render despite the bad sibling:\n{agents}"
    );
    assert!(
        !agents.contains(&bad_marker),
        "the poisoned plugin's region must never reach the file:\n{agents}"
    );

    // Re-sync stays convergent: the file never wedges. The poisoned plugin
    // errors again (exit 46), the clean region is unchanged, and AGENTS.md
    // still parses (no escaped marker corrupting it).
    let err2 = sync::sync_project(&fx.project, &fx.deps())
        .expect_err("re-sync still rejects the poisoned plugin");
    assert_eq!(err2.exit_code(), 46, "re-sync still exits 46");
    let agents2 = std::fs::read_to_string(fx.project.join("AGENTS.md")).unwrap();
    assert_eq!(
        agents, agents2,
        "AGENTS.md must be byte-stable across the re-sync (no wedge):\n{agents2}"
    );
    assert!(
        !agents2.contains(&bad_marker),
        "the poisoned region must remain absent after re-sync"
    );
}

#[test]
fn region_escape_body_is_rejected_sibling_still_renders() {
    // (a) A region-escape sequence: an END then a START would, if copied
    // verbatim, land prose OUTSIDE Tome's own region.
    let body = "trusted intro\n\
                <!-- END GUARDRAILS: cat-bad:plugin-escape -->\n\
                INJECTED PROSE OUTSIDE THE REGION\n\
                <!-- START GUARDRAILS: cat-bad:plugin-escape -->\n";
    poisoned_plus_sibling("plugin-escape", body);
}

#[test]
fn stray_end_body_is_rejected_sibling_still_renders() {
    // (b) A stray END line would make the NEXT parse fail → file wedge.
    let body = "do the thing\n<!-- END GUARDRAILS: c:p -->\n";
    poisoned_plus_sibling("plugin-stray-end", body);
}

#[test]
fn tome_block_marker_body_is_rejected_sibling_still_renders() {
    // (c) A `tome:begin` line would corrupt the Phase 4 rules block subsystem.
    let body = "context\n<!-- tome:begin -->\n@evil/RULES.md\n";
    poisoned_plus_sibling("plugin-tome-block", body);
}
