//! Phase 6 / US3 — guardrails render reconciliation (T090).
//!
//! Drives the real harness modules through `sync_project`:
//!
//! - A plugin shipping `GUARDRAILS.md` (no JSON hooks) renders a
//!   `<catalog>:<plugin>` region into `CLAUDE.md`, the shared `AGENTS.md`,
//!   and the Cursor sibling `.cursor/rules/TOME_GUARDRAILS.md`.
//! - Two guardrails-shipping plugins produce two regions in lexicographic
//!   `<catalog>:<plugin>` order.
//! - Disabling one plugin removes only its region.
//! - A re-sync with unchanged content rewrites nothing (mtime idempotence).
//! - The Cursor sibling is deleted entirely when the last contributor leaves.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

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
            dry_run: false,
        }
    }
}

/// Seed a manifest-less catalog enrolment plus an on-disk
/// `hooks/GUARDRAILS.md`, returning the catalog URL. The file lives at
/// `<cache_dir_for(url)>/<plugin>/hooks/GUARDRAILS.md` so the guardrails
/// pass's `plugin_root_dir` (manifest-less fallback) finds it.
fn seed_guardrails_source(paths: &tome::paths::Paths, plugin: &str, body: &str) -> String {
    let url = format!("https://example.test/{plugin}.git");
    let cache = paths.cache_dir_for(&url);
    let hooks_dir = cache.join(plugin).join("hooks");
    std::fs::create_dir_all(&hooks_dir).expect("create hooks dir");
    std::fs::write(hooks_dir.join("GUARDRAILS.md"), body).expect("write GUARDRAILS.md");
    url
}

/// Insert an enabled `skill`-kind row for `(catalog, plugin)` so the plugin
/// shows up in the workspace's enabled-plugin enumeration.
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

fn mtime(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .unwrap_or_else(|e| panic!("stat {}: {e}", path.display()))
        .modified()
        .expect("modified time")
}

#[test]
fn region_renders_in_claude_md_shared_agents_and_cursor_sibling() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![
        Box::new(tome::harness::claude_code::CLAUDE_CODE),
        Box::new(tome::harness::codex::CODEX),
        Box::new(tome::harness::cursor::CURSOR),
    ]);

    let fx = Fixture::build("test-ws", "\"claude-code\", \"codex\", \"cursor\"");
    let url = seed_guardrails_source(&fx.paths, "plugin-a", "Be careful with deletes.\n");
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "test-ws", "cat", &url, "main")
        .expect("enrol catalog");
    drop(conn);
    insert_enabled_skill_row(&fx.paths, "test-ws", "cat", "plugin-a");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    let marker = "<!-- START GUARDRAILS: cat:plugin-a -->";

    // CLAUDE.md (claude-code, no hooks → not suppressed).
    let claude = std::fs::read_to_string(fx.project.join("CLAUDE.md")).unwrap();
    assert!(claude.contains(marker), "CLAUDE.md region:\n{claude}");
    assert!(claude.contains("Be careful with deletes."));

    // Shared AGENTS.md (codex).
    let agents = std::fs::read_to_string(fx.project.join("AGENTS.md")).unwrap();
    assert!(agents.contains(marker), "AGENTS.md region:\n{agents}");

    // Cursor standalone sibling (distinct from TOME_SKILLS.md).
    let sibling = fx.project.join(".cursor/rules/TOME_GUARDRAILS.md");
    assert!(sibling.is_file(), "Cursor sibling must exist");
    let sib_body = std::fs::read_to_string(&sibling).unwrap();
    assert!(
        sib_body.contains(marker),
        "Cursor sibling region:\n{sib_body}"
    );
}

#[test]
fn two_plugins_render_two_regions_in_lexicographic_order() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::codex::CODEX)]);

    let fx = Fixture::build("test-ws", "\"codex\"");
    // Two plugins under distinct catalogs, ensuring lexicographic key order:
    // "cat-a:plugin-alpha" < "cat-b:plugin-zeta".
    let url_z = seed_guardrails_source(&fx.paths, "plugin-zeta", "zeta rules\n");
    let url_a = seed_guardrails_source(&fx.paths, "plugin-alpha", "alpha rules\n");
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "test-ws", "cat-a", &url_a, "main")
        .expect("enrol a");
    tome::index::workspace_catalogs::insert(&conn, "test-ws", "cat-b", &url_z, "main")
        .expect("enrol b");
    drop(conn);
    insert_enabled_skill_row(&fx.paths, "test-ws", "cat-a", "plugin-alpha");
    insert_enabled_skill_row(&fx.paths, "test-ws", "cat-b", "plugin-zeta");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    let agents = std::fs::read_to_string(fx.project.join("AGENTS.md")).unwrap();
    let alpha = agents
        .find("cat-a:plugin-alpha")
        .expect("alpha region present");
    let zeta = agents
        .find("cat-b:plugin-zeta")
        .expect("zeta region present");
    assert!(
        alpha < zeta,
        "alpha region must precede zeta (lexicographic order):\n{agents}"
    );
    assert_eq!(
        agents.matches("<!-- START GUARDRAILS:").count(),
        2,
        "exactly two regions:\n{agents}"
    );
}

#[test]
fn disabling_one_plugin_removes_only_its_region() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::codex::CODEX)]);

    let fx = Fixture::build("test-ws", "\"codex\"");
    let url_a = seed_guardrails_source(&fx.paths, "plugin-a", "a rules\n");
    let url_b = seed_guardrails_source(&fx.paths, "plugin-b", "b rules\n");
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "test-ws", "cat-a", &url_a, "main")
        .expect("enrol a");
    tome::index::workspace_catalogs::insert(&conn, "test-ws", "cat-b", &url_b, "main")
        .expect("enrol b");
    drop(conn);
    insert_enabled_skill_row(&fx.paths, "test-ws", "cat-a", "plugin-a");
    insert_enabled_skill_row(&fx.paths, "test-ws", "cat-b", "plugin-b");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync 1");
    let agents = std::fs::read_to_string(fx.project.join("AGENTS.md")).unwrap();
    assert!(agents.contains("cat-a:plugin-a") && agents.contains("cat-b:plugin-b"));

    // Disable plugin-b.
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    conn.execute(
        "DELETE FROM workspace_skills WHERE skill_id IN
            (SELECT id FROM skills WHERE plugin = 'plugin-b')",
        [],
    )
    .expect("disable plugin-b");
    drop(conn);

    sync::sync_project(&fx.project, &fx.deps()).expect("sync 2");
    let agents = std::fs::read_to_string(fx.project.join("AGENTS.md")).unwrap();
    assert!(
        agents.contains("cat-a:plugin-a"),
        "plugin-a region survives:\n{agents}"
    );
    assert!(
        !agents.contains("cat-b:plugin-b"),
        "plugin-b region removed:\n{agents}"
    );
}

#[test]
fn resync_unchanged_content_rewrites_nothing() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::codex::CODEX)]);

    let fx = Fixture::build("test-ws", "\"codex\"");
    let url = seed_guardrails_source(&fx.paths, "plugin-a", "stable rules\n");
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "test-ws", "cat", &url, "main").expect("enrol");
    drop(conn);
    insert_enabled_skill_row(&fx.paths, "test-ws", "cat", "plugin-a");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync 1");
    let agents = fx.project.join("AGENTS.md");
    let m1 = mtime(&agents);

    std::thread::sleep(Duration::from_millis(1500));
    let outcome2 = sync::sync_project(&fx.project, &fx.deps()).expect("sync 2");
    assert!(
        outcome2
            .added
            .iter()
            .chain(&outcome2.updated)
            .chain(&outcome2.removed)
            .all(|c| c.subsystem != sync::SyncSubsystem::Guardrails),
        "idempotent re-sync must not touch guardrails"
    );
    assert_eq!(mtime(&agents), m1, "AGENTS.md mtime must not advance");
}

#[test]
fn cursor_sibling_deleted_when_last_contributor_leaves() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::cursor::CURSOR)]);

    let fx = Fixture::build("test-ws", "\"cursor\"");
    let url = seed_guardrails_source(&fx.paths, "plugin-a", "cursor rules\n");
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "test-ws", "cat", &url, "main").expect("enrol");
    drop(conn);
    insert_enabled_skill_row(&fx.paths, "test-ws", "cat", "plugin-a");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync 1");
    let sibling = fx.project.join(".cursor/rules/TOME_GUARDRAILS.md");
    assert!(sibling.is_file(), "sibling created on first sync");

    // Disable the only contributor.
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    conn.execute(
        "DELETE FROM workspace_skills WHERE skill_id IN
            (SELECT id FROM skills WHERE plugin = 'plugin-a')",
        [],
    )
    .expect("disable plugin-a");
    drop(conn);

    sync::sync_project(&fx.project, &fx.deps()).expect("sync 2");
    assert!(
        !sibling.exists(),
        "Cursor sibling must be deleted when no plugin contributes (FR-015)"
    );
}

/// T3-4 — gemini guardrails target: no `AGENTS.md` present → the region lands
/// in `GEMINI.md`; with `AGENTS.md` present → the shared `AGENTS.md` wins.
#[test]
fn gemini_target_falls_back_to_gemini_md_then_prefers_agents_md() {
    // ----- branch 1: no AGENTS.md, GEMINI.md present → GEMINI.md -----
    {
        let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::gemini::GEMINI)]);

        let fx = Fixture::build("test-ws", "\"gemini\"");
        // Gemini's resolver prefers AGENTS.md, then GEMINI.md. With no
        // AGENTS.md but a present GEMINI.md, the region must land in GEMINI.md.
        std::fs::write(fx.project.join("GEMINI.md"), "# gemini context\n").expect("seed GEMINI.md");
        let url = seed_guardrails_source(&fx.paths, "plugin-g", "gemini rules\n");
        let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
        tome::index::workspace_catalogs::insert(&conn, "test-ws", "cat", &url, "main")
            .expect("enrol");
        drop(conn);
        insert_enabled_skill_row(&fx.paths, "test-ws", "cat", "plugin-g");

        sync::sync_project(&fx.project, &fx.deps()).expect("sync");

        let marker = "<!-- START GUARDRAILS: cat:plugin-g -->";
        assert!(
            !fx.project.join("AGENTS.md").exists(),
            "no AGENTS.md should have been created"
        );
        let gemini = std::fs::read_to_string(fx.project.join("GEMINI.md"))
            .expect("GEMINI.md must be the resolved target");
        assert!(
            gemini.contains("# gemini context") && gemini.contains(marker),
            "region must land in the pre-existing GEMINI.md:\n{gemini}"
        );
    }

    // ----- branch 2: AGENTS.md present → shared AGENTS.md -----
    {
        let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::gemini::GEMINI)]);

        let fx = Fixture::build("test-ws", "\"gemini\"");
        // Pre-create AGENTS.md so gemini's first-existing-wins resolver picks it.
        std::fs::write(fx.project.join("AGENTS.md"), "# project agents\n").expect("seed AGENTS.md");
        let url = seed_guardrails_source(&fx.paths, "plugin-g", "gemini rules\n");
        let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
        tome::index::workspace_catalogs::insert(&conn, "test-ws", "cat", &url, "main")
            .expect("enrol");
        drop(conn);
        insert_enabled_skill_row(&fx.paths, "test-ws", "cat", "plugin-g");

        sync::sync_project(&fx.project, &fx.deps()).expect("sync");

        let marker = "<!-- START GUARDRAILS: cat:plugin-g -->";
        let agents = std::fs::read_to_string(fx.project.join("AGENTS.md")).unwrap();
        assert!(
            agents.contains("# project agents") && agents.contains(marker),
            "region must land in the pre-existing shared AGENTS.md:\n{agents}"
        );
        assert!(
            !fx.project.join("GEMINI.md").exists(),
            "GEMINI.md must NOT be created when AGENTS.md already exists"
        );
    }
}

/// T3-5 — two plugins on the Cursor standalone sibling: both regions present,
/// individually wrapped, lexicographic order, exactly two START markers.
#[test]
fn cursor_sibling_renders_two_regions_lexicographically() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::cursor::CURSOR)]);

    let fx = Fixture::build("test-ws", "\"cursor\"");
    let url_z = seed_guardrails_source(&fx.paths, "plugin-zeta", "zeta rules\n");
    let url_a = seed_guardrails_source(&fx.paths, "plugin-alpha", "alpha rules\n");
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "test-ws", "cat-a", &url_a, "main")
        .expect("enrol a");
    tome::index::workspace_catalogs::insert(&conn, "test-ws", "cat-b", &url_z, "main")
        .expect("enrol b");
    drop(conn);
    insert_enabled_skill_row(&fx.paths, "test-ws", "cat-a", "plugin-alpha");
    insert_enabled_skill_row(&fx.paths, "test-ws", "cat-b", "plugin-zeta");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    let sibling = fx.project.join(".cursor/rules/TOME_GUARDRAILS.md");
    let body = std::fs::read_to_string(&sibling).expect("sibling exists");

    let alpha = body
        .find("cat-a:plugin-alpha")
        .expect("alpha region present");
    let zeta = body.find("cat-b:plugin-zeta").expect("zeta region present");
    assert!(
        alpha < zeta,
        "alpha region must precede zeta (lexicographic order):\n{body}"
    );
    assert_eq!(
        body.matches("<!-- START GUARDRAILS:").count(),
        2,
        "exactly two regions in the sibling:\n{body}"
    );
    // Each region is individually wrapped with its own END.
    assert!(body.contains("<!-- END GUARDRAILS: cat-a:plugin-alpha -->"));
    assert!(body.contains("<!-- END GUARDRAILS: cat-b:plugin-zeta -->"));
}

/// T3-6 — changed-source overwrite-in-place: sync, rewrite the `GUARDRAILS.md`
/// body, sync again → the region is updated in place (exactly one START), with
/// the new body, and the file's mtime advances.
#[test]
fn changed_source_overwrites_region_in_place() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::codex::CODEX)]);

    let fx = Fixture::build("test-ws", "\"codex\"");
    let url = seed_guardrails_source(&fx.paths, "plugin-a", "original body\n");
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "test-ws", "cat", &url, "main").expect("enrol");
    drop(conn);
    insert_enabled_skill_row(&fx.paths, "test-ws", "cat", "plugin-a");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync 1");
    let agents = fx.project.join("AGENTS.md");
    let m1 = mtime(&agents);
    let first = std::fs::read_to_string(&agents).unwrap();
    assert!(first.contains("original body"));

    // Rewrite the source body, then re-sync.
    std::thread::sleep(Duration::from_millis(1500));
    seed_guardrails_source(&fx.paths, "plugin-a", "updated body\n");
    sync::sync_project(&fx.project, &fx.deps()).expect("sync 2");

    let second = std::fs::read_to_string(&agents).unwrap();
    assert!(second.contains("updated body"), "body updated:\n{second}");
    assert!(
        !second.contains("original body"),
        "old body replaced:\n{second}"
    );
    assert_eq!(
        second
            .matches("<!-- START GUARDRAILS: cat:plugin-a -->")
            .count(),
        1,
        "exactly one START marker — overwritten in place, not duplicated:\n{second}"
    );
    assert!(
        mtime(&agents) > m1,
        "mtime must advance after the body changed"
    );
}

/// T3-7 — verbatim-body fidelity: a body with frontmatter-looking lines, a
/// heading, an `@include`, and trailing whitespace (NO marker lines, which are
/// now rejected by B-1) is reproduced byte-for-byte between the markers.
#[test]
fn verbatim_body_is_reproduced_byte_for_byte() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::codex::CODEX)]);

    let fx = Fixture::build("test-ws", "\"codex\"");
    // Parseable-looking but marker-free content, including trailing whitespace
    // on a line and no trailing newline at the end of the body.
    let source_body = "---\ntitle: Guardrails\npriority: 9\n---\n# Be careful\n@team/RULES.md\nTrailing ws here.   ";
    let url = seed_guardrails_source(&fx.paths, "plugin-a", source_body);
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "test-ws", "cat", &url, "main").expect("enrol");
    drop(conn);
    insert_enabled_skill_row(&fx.paths, "test-ws", "cat", "plugin-a");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    let agents = std::fs::read_to_string(fx.project.join("AGENTS.md")).unwrap();
    let begin = "<!-- START GUARDRAILS: cat:plugin-a -->\n";
    let end = "\n<!-- END GUARDRAILS: cat:plugin-a -->";
    let start_idx = agents.find(begin).expect("START marker present") + begin.len();
    let end_idx = agents[start_idx..].find(end).expect("END marker present") + start_idx;
    let between = &agents[start_idx..end_idx];
    assert_eq!(
        between, source_body,
        "body between markers must equal the source verbatim:\n--- got ---\n{between}\n--- want ---\n{source_body}"
    );
}
