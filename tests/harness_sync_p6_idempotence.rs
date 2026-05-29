//! Phase 6 polish (T149) — whole-phase re-sync idempotence with mtime capture.
//!
//! The existing idempotence proofs are PER-SINK: `harness_sync_stub.rs`'s
//! `real_hooks_merge_idempotence_and_removal_for_claude_code` and
//! `native_agents_emit_orphan_removal_and_idempotence`, plus
//! `guardrails_render.rs`'s `resync_unchanged_content_rewrites_nothing`. None
//! drives all THREE Phase 6 sinks (hooks + guardrails + native agents) through
//! a single sync and proves the second run is a byte-for-byte no-op across all
//! of them at once. A sink-ordering refactor that accidentally re-wrote one
//! sink on re-sync could slip past every per-sink test; this closes that gap.
//!
//! ## One harness, three sinks
//!
//! A single `StubHarness` configured `RealJson` + `with_hook_settings()` +
//! `with_native_agents` drives all three sinks at once. Its default guardrails
//! target is an in-file region on the rules-file path (`STUB_RULES.md`,
//! `suppress_if_hooks_present: false`), so the guardrails region renders even
//! though the seeded plugin also ships `hooks.json`. One enabled plugin ships
//! all three component sources; a single `sync_project` exercises every sink.
//!
//! ## Why the mtime check is load-bearing
//!
//! An empty-`added`/`updated` outcome alone would NOT catch a "rewrites
//! identical bytes" regression (a writer that re-emits the same content but
//! still touches the file, classifying it `LeftAlone`). Capturing each sink
//! output's mtime before a >1s sleep and asserting it is unchanged after the
//! second sync catches exactly that class of bug.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::harness::sync::{self, SyncDeps, SyncSubsystem};
use tome::harness::{AgentFormat, HooksStrategy, StubHarness};
use tome::workspace::WorkspaceName;

/// Process-global mutex serialising every test in this file — the
/// `HARNESS_MODULES_OVERRIDE` slot is process-wide and cargo runs tests on
/// multiple threads. Mirrors the `harness_sync_stub.rs` convention.
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
        // The Inline rules block reads this; present so the rules pass is also
        // idempotent (the guardrails region shares the rules-file target).
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

fn mtime(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .unwrap_or_else(|e| panic!("stat {}: {e}", path.display()))
        .modified()
        .expect("modified time")
}

/// Seed a manifest-less catalog enrolment plus an on-disk plugin
/// `hooks/hooks.json`. The file lives at
/// `<cache_dir_for(url)>/<plugin>/hooks/hooks.json` so the hooks pass's
/// manifest-less `plugin_root_dir` fallback finds it. Returns the catalog URL.
fn seed_hooks_source(paths: &tome::paths::Paths, plugin: &str, body: &str) -> String {
    let url = format!("https://example.test/{plugin}.git");
    let hooks_dir = paths.cache_dir_for(&url).join(plugin).join("hooks");
    std::fs::create_dir_all(&hooks_dir).expect("create hooks source dir");
    std::fs::write(hooks_dir.join("hooks.json"), body).expect("write source hooks.json");
    url
}

/// Seed a `hooks/GUARDRAILS.md` under the same plugin-root cache dir as
/// [`seed_hooks_source`]. The body must contain NO marker-shaped lines (the
/// B-1 fail-closed validator would otherwise reject it, exit 46).
fn seed_guardrails_source(paths: &tome::paths::Paths, plugin: &str, body: &str) {
    let url = format!("https://example.test/{plugin}.git");
    let hooks_dir = paths.cache_dir_for(&url).join(plugin).join("hooks");
    std::fs::create_dir_all(&hooks_dir).expect("create hooks source dir");
    std::fs::write(hooks_dir.join("GUARDRAILS.md"), body).expect("write GUARDRAILS.md");
}

/// Seed an `agents/<name>.md` under the same plugin-root cache dir so
/// `resolve_entry_body_path`'s manifest-less fallback finds it.
fn seed_agent_source(paths: &tome::paths::Paths, plugin: &str, name: &str, body: &str) {
    let url = format!("https://example.test/{plugin}.git");
    let agent_dir = paths.cache_dir_for(&url).join(plugin).join("agents");
    std::fs::create_dir_all(&agent_dir).expect("create agent source dir");
    std::fs::write(agent_dir.join(format!("{name}.md")), body).expect("write source agent");
}

/// Insert an enabled `agent`-kind row for `(catalog, plugin, name)`. An
/// enabled row of ANY kind also makes the plugin appear in
/// `enabled_plugins_for_workspace` (DISTINCT catalog/plugin), so this single
/// row drives the hooks + guardrails enumeration as well as the agent path.
fn insert_enabled_agent_row(
    paths: &tome::paths::Paths,
    workspace: &str,
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
    let skill_id: i64 = conn
        .query_row(
            "SELECT id FROM skills WHERE catalog=?1 AND plugin=?2 AND kind='agent' AND name=?3",
            rusqlite::params![catalog, plugin, name],
            |r| r.get(0),
        )
        .expect("agent id");
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
    .expect("enrol agent");
}

#[test]
fn all_three_p6_sinks_resync_is_byte_for_byte_noop() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    // A single stub driving all three Phase 6 sinks: RealJson hooks with a
    // settings path, native agents, and the default in-file guardrails region
    // (no hooks-suppression) on the rules-file target.
    let _guard = HarnessModulesGuard::install(vec![Box::new(
        StubHarness::default()
            .with_hooks_strategy(HooksStrategy::RealJson)
            .with_hook_settings()
            .with_native_agents(AgentFormat::MarkdownYaml),
    )]);

    let fx = Fixture::build("test-workspace", "\"stub\"");

    // One plugin shipping all three component sources. The `${CLAUDE_PLUGIN_ROOT}`
    // token makes the hooks rewrite observable + deterministic; the guardrails
    // body is marker-free so the B-1 validator accepts it.
    let url = seed_hooks_source(
        &fx.paths,
        "plugin-a",
        r#"{ "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/guard.sh" } ] } ] }"#,
    );
    seed_guardrails_source(&fx.paths, "plugin-a", "Be careful with deletes.\n");
    seed_agent_source(
        &fx.paths,
        "plugin-a",
        "reviewer",
        "---\nname: reviewer\ndescription: Reviews code.\n---\nYou review code.\n",
    );

    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "test-workspace", "cat", &url, "main")
        .expect("enrol catalog");
    drop(conn);
    insert_enabled_agent_row(&fx.paths, "test-workspace", "cat", "plugin-a", "reviewer");

    // ----- sync 1: every sink lands -----
    let outcome1 = sync::sync_project(&fx.project, &fx.deps()).expect("sync 1");

    let hooks_path = fx.project.join(".stub/settings.local.json");
    let guardrails_path = fx.project.join("STUB_RULES.md");
    let agent_path = fx.project.join(".stub/agents/plugin-a__reviewer.md");

    assert!(
        hooks_path.is_file(),
        "hooks settings file created on sync 1"
    );
    assert!(
        guardrails_path.is_file(),
        "guardrails target (rules-file) created on sync 1"
    );
    assert!(agent_path.is_file(), "agent file created on sync 1");

    // Each Phase 6 sink recorded at least one change on the first sync, so the
    // no-op assertion below is meaningful (we are not idempotent because a sink
    // never fired).
    for sink in [
        SyncSubsystem::Hooks,
        SyncSubsystem::Guardrails,
        SyncSubsystem::Agents,
    ] {
        assert!(
            outcome1
                .added
                .iter()
                .chain(&outcome1.updated)
                .any(|c| c.subsystem == sink),
            "sink {sink:?} must record a change on the first sync; outcome: {outcome1:?}",
        );
    }

    // Capture every sink output's mtime, then sleep past the coarsest common
    // filesystem mtime granularity (1s on some filesystems; 1100ms is the
    // established margin used by the scaffold).
    let hooks_mtime = mtime(&hooks_path);
    let guardrails_mtime = mtime(&guardrails_path);
    let agent_mtime = mtime(&agent_path);
    std::thread::sleep(Duration::from_millis(1100));

    // ----- sync 2: must be a byte-for-byte no-op across all three sinks -----
    let outcome2 = sync::sync_project(&fx.project, &fx.deps()).expect("sync 2");

    for sink in [
        SyncSubsystem::Hooks,
        SyncSubsystem::Guardrails,
        SyncSubsystem::Agents,
    ] {
        assert!(
            outcome2.added.iter().all(|c| c.subsystem != sink),
            "sink {sink:?} `added` must be empty on re-sync; outcome: {outcome2:?}",
        );
        assert!(
            outcome2.updated.iter().all(|c| c.subsystem != sink),
            "sink {sink:?} `updated` must be empty on re-sync; outcome: {outcome2:?}",
        );
    }

    // The load-bearing half: even a writer that re-emits identical bytes would
    // advance these mtimes. They must be unchanged.
    assert_eq!(
        mtime(&hooks_path),
        hooks_mtime,
        "hooks settings mtime must not advance on idempotent re-sync"
    );
    assert_eq!(
        mtime(&guardrails_path),
        guardrails_mtime,
        "guardrails target mtime must not advance on idempotent re-sync"
    );
    assert_eq!(
        mtime(&agent_path),
        agent_mtime,
        "agent file mtime must not advance on idempotent re-sync"
    );
}
