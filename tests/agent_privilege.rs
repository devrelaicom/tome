//! T122 / T123 — `strip_plugin_agent_privileges` end-to-end (Phase 6 / US5).
//!
//! Drives [`tome::harness::sync::sync_project`] against the real
//! [`tome::harness::claude_code::CLAUDE_CODE`] harness (and a native
//! `StubHarness` for the negative case), asserting the contract's
//! `settings-p6.md` § `strip_plugin_agent_privileges` rows (FR-050/052/053):
//!
//! * (a) passthrough by default — the emitted Claude Code agent carries
//!   `hooks` / `mcpServers` / `permissionMode` (the capability advantage).
//! * (b) strip when set (workspace OR global `strip_plugin_agent_privileges
//!   = true`) — the same agent is emitted WITHOUT the three fields.
//! * (c) strip is a no-op for an agent carrying none of the three.
//! * (d) the setting has no effect on a non-Claude-Code harness — the stub
//!   never carries the privileged fields regardless of the flag.
//!
//! The strip is applied to a per-emission CLONE of the canonical agent, so
//! the agent SOURCE is never mutated; (b) re-reads the source `.md` after the
//! strip sync and asserts it is byte-for-byte unchanged — the US5 doctor
//! privilege audit still sees the original privileged frontmatter.

mod common;

use std::path::PathBuf;
use std::sync::Mutex;

use common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::harness::AgentFormat;
use tome::harness::StubHarness;
use tome::harness::claude_code::CLAUDE_CODE;
use tome::harness::sync::{self, SyncDeps};
use tome::workspace::WorkspaceName;

/// Process-global mutex serialising every test in this file —
/// `HARNESS_MODULES_OVERRIDE` is a single slot and cargo runs `#[test]`
/// cases on multiple threads.
static OVERRIDE_MUTEX: Mutex<()> = Mutex::new(());

/// The privileged frontmatter keys the strip removes (Claude Code spelling).
const PRIVILEGED_KEYS: [&str; 3] = ["hooks:", "mcpServers:", "permissionMode:"];

struct Fixture {
    _home: TempDir,
    paths: tome::paths::Paths,
    project: PathBuf,
    workspace: WorkspaceName,
}

impl Fixture {
    /// Build a project bound to `workspace_name`, with the project marker
    /// declaring `harnesses = [<harnesses>]`. Optional workspace / global
    /// settings TOML bodies are written when supplied so the strip flag can be
    /// declared at any scope.
    fn build(
        workspace_name: &str,
        harnesses: &str,
        workspace_settings_extra: Option<&str>,
        global_settings: Option<&str>,
    ) -> Self {
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
        .expect("write marker config");

        if let Some(extra) = workspace_settings_extra {
            let ws_dir = paths.workspace_dir(&workspace);
            std::fs::create_dir_all(&ws_dir).expect("create workspace dir");
            std::fs::write(
                ws_dir.join("settings.toml"),
                format!("name = \"{workspace_name}\"\n{extra}\n"),
            )
            .expect("write workspace settings");
        }

        if let Some(global) = global_settings {
            std::fs::write(&paths.global_settings_file, format!("{global}\n"))
                .expect("write global settings");
        }

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

/// Seed an on-disk source agent `.md` under a manifest-less catalog cache so
/// `resolve_entry_body_path` finds it. Returns the catalog URL.
fn seed_agent_source(paths: &tome::paths::Paths, plugin: &str, name: &str, body: &str) -> String {
    let url = format!("https://example.test/{plugin}.git");
    let cache = paths.cache_dir_for(&url);
    let agent_dir = cache.join(plugin).join("agents");
    std::fs::create_dir_all(&agent_dir).expect("create agent source dir");
    std::fs::write(agent_dir.join(format!("{name}.md")), body).expect("write source agent");
    url
}

/// Enrol `catalog` for `workspace` and insert an enabled `agent`-kind row for
/// `(catalog, plugin, name)` pointing at the catalog-relative source path.
fn enrol_and_enable_agent(
    paths: &tome::paths::Paths,
    workspace: &str,
    catalog: &str,
    url: &str,
    plugin: &str,
    name: &str,
) {
    let conn = rusqlite::Connection::open(&paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, workspace, catalog, url, "main")
        .expect("enrol catalog");
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

/// A privileged source agent carrying all three privileged frontmatter fields.
const PRIVILEGED_AGENT: &str = "---\n\
name: reviewer\n\
description: Reviews code\n\
hooks:\n  PreToolUse:\n    - matcher: Bash\n\
mcpServers:\n  foo:\n    command: x\n\
permissionMode: ask\n\
---\n\
You review code.\n";

/// A plain source agent carrying NONE of the three privileged fields.
const PLAIN_AGENT: &str = "---\nname: builder\ndescription: Builds things\n---\nYou build.\n";

fn assert_has_privileged(body: &str) {
    for key in PRIVILEGED_KEYS {
        assert!(
            body.contains(key),
            "expected `{key}` in emitted agent:\n{body}"
        );
    }
}

fn assert_no_privileged(body: &str) {
    for key in PRIVILEGED_KEYS {
        assert!(
            !body.contains(key),
            "did NOT expect `{key}` in emitted agent:\n{body}"
        );
    }
}

// ---------------------------------------------------------------------------
// (a) passthrough by default.
// ---------------------------------------------------------------------------

#[test]
fn passthrough_by_default() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(CLAUDE_CODE)]);

    let fx = Fixture::build("ws", "\"claude-code\"", None, None);
    let url = seed_agent_source(&fx.paths, "plugin-a", "reviewer", PRIVILEGED_AGENT);
    enrol_and_enable_agent(&fx.paths, "ws", "cat-a", &url, "plugin-a", "reviewer");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    let emitted = fx.project.join(".claude/agents/plugin-a__reviewer.md");
    let body = std::fs::read_to_string(&emitted).expect("read emitted agent");
    // The privilege passthrough is the default (FR-050) — the three fields
    // survive into the emitted Claude Code agent file.
    assert_has_privileged(&body);
}

// ---------------------------------------------------------------------------
// (b) strip when set — workspace scope.
// ---------------------------------------------------------------------------

#[test]
fn strip_removes_three_fields_workspace_scope() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(CLAUDE_CODE)]);

    let fx = Fixture::build(
        "ws",
        "\"claude-code\"",
        Some("strip_plugin_agent_privileges = true"),
        None,
    );
    let url = seed_agent_source(&fx.paths, "plugin-a", "reviewer", PRIVILEGED_AGENT);
    enrol_and_enable_agent(&fx.paths, "ws", "cat-a", &url, "plugin-a", "reviewer");

    // The on-disk source path for the post-sync audit-invariance assertion.
    let source = fx
        .paths
        .cache_dir_for(&url)
        .join("plugin-a")
        .join("agents")
        .join("reviewer.md");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    let emitted = fx.project.join(".claude/agents/plugin-a__reviewer.md");
    let body = std::fs::read_to_string(&emitted).expect("read emitted agent");
    // Strip resolved true at the workspace scope → the three fields are gone
    // from the emitted file (FR-052).
    assert_no_privileged(&body);
    // Non-privileged fields survive the strip.
    assert!(body.contains("name: reviewer"), "name survives:\n{body}");

    // Audit invariance: the SOURCE agent is untouched — the strip mutated only
    // the per-emission clone, so the US5 doctor privilege audit still reads the
    // original privileged frontmatter from the source.
    let source_body = std::fs::read_to_string(&source).expect("read source agent");
    assert_eq!(
        source_body, PRIVILEGED_AGENT,
        "the source agent must NOT be mutated by the strip",
    );
}

// ---------------------------------------------------------------------------
// (b') strip when set — global scope.
// ---------------------------------------------------------------------------

#[test]
fn strip_removes_three_fields_global_scope() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(CLAUDE_CODE)]);

    // Declared org-wide at the global scope; project + workspace leave the key
    // absent so the first-declarer-wins walk falls through to global.
    let fx = Fixture::build(
        "ws",
        "\"claude-code\"",
        None,
        Some("strip_plugin_agent_privileges = true"),
    );
    let url = seed_agent_source(&fx.paths, "plugin-a", "reviewer", PRIVILEGED_AGENT);
    enrol_and_enable_agent(&fx.paths, "ws", "cat-a", &url, "plugin-a", "reviewer");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    let emitted = fx.project.join(".claude/agents/plugin-a__reviewer.md");
    let body = std::fs::read_to_string(&emitted).expect("read emitted agent");
    assert_no_privileged(&body);
}

// ---------------------------------------------------------------------------
// (c) strip is a no-op for an agent carrying none of the three fields.
// ---------------------------------------------------------------------------

#[test]
fn strip_noop_when_none() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(CLAUDE_CODE)]);

    let fx = Fixture::build(
        "ws",
        "\"claude-code\"",
        Some("strip_plugin_agent_privileges = true"),
        None,
    );
    let url = seed_agent_source(&fx.paths, "plugin-a", "builder", PLAIN_AGENT);
    enrol_and_enable_agent(&fx.paths, "ws", "cat-a", &url, "plugin-a", "builder");

    // The strip is enabled but the agent carries none of the privileged fields.
    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    let emitted = fx.project.join(".claude/agents/plugin-a__builder.md");
    let body = std::fs::read_to_string(&emitted).expect("read emitted agent");
    // No privileged fields to begin with → still none, and the agent emits
    // normally (the strip did not corrupt the otherwise-valid agent).
    assert_no_privileged(&body);
    assert!(body.contains("name: builder"), "name present:\n{body}");
    assert!(body.contains("You build."), "body present:\n{body}");
}

// ---------------------------------------------------------------------------
// (d) the setting has no effect on a non-Claude-Code harness.
// ---------------------------------------------------------------------------

#[test]
fn strip_claude_code_only() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    // A native StubHarness standing in for codex/cursor/opencode: it never
    // carries the privileged fields (its translation drops them), so the strip
    // flag must make no observable difference to its emission.
    let _guard = HarnessModulesGuard::install(vec![Box::new(
        StubHarness::default().with_native_agents(AgentFormat::MarkdownYaml),
    )]);

    let fx = Fixture::build(
        "ws",
        "\"stub\"",
        Some("strip_plugin_agent_privileges = true"),
        None,
    );
    // Even a privileged SOURCE agent: the stub translation never emits the
    // three fields, so the strip is a no-op for it.
    let url = seed_agent_source(&fx.paths, "plugin-a", "reviewer", PRIVILEGED_AGENT);
    enrol_and_enable_agent(&fx.paths, "ws", "cat-a", &url, "plugin-a", "reviewer");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    let emitted = fx.project.join(".stub/agents/plugin-a__reviewer.md");
    let body = std::fs::read_to_string(&emitted).expect("read emitted stub agent");
    // The stub never carried the privileged fields regardless of the flag.
    assert_no_privileged(&body);
}
