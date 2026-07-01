//! Phase 6 polish (T150) — enable -> reconcile ALL sinks -> assert hooks +
//! guardrails + agents + persona end-to-end (the HAPPY whole flow).
//!
//! The per-US suites each prove one surface in isolation: `harness_sync_stub.rs`
//! (hooks + agents through sync), `guardrails_render.rs` (rendered regions),
//! `personas.rs` (the persona registry). None proves that ONE enabled plugin,
//! shipping all three component sources, lights up all FOUR user-facing
//! surfaces from a single workspace state. This is that proof.
//!
//! ## Shared on-disk state
//!
//! `sync_project` and `PromptRegistry::build_for_workspace` read the SAME
//! `skills` rows and the SAME URL-hashed catalog cache. So one
//! `insert_enabled_agent_row` (which also makes the plugin appear in the
//! `enabled_plugins` enumeration that drives hooks + guardrails) plus
//! manifest-less `hooks.json` / `GUARDRAILS.md` / `agents/<name>.md` sources
//! feed every surface. A single `StubHarness` configured `RealJson` +
//! `with_hook_settings()` + `with_native_agents` + its default in-file
//! guardrails region drives all three sinks in one sync.
//!
//! The persona toggle is resolved END-TO-END from an on-disk workspace
//! `settings.toml` (`expose_agents_as_personas = true`) via
//! `resolve_expose_personas`, then threaded into the registry build — proving
//! the setting, not a literal `true`, drives the persona surface.
//!
//! This file holds the HAPPY path only; failure/precedence lives in
//! `harness_sync_p6_first_error.rs`.

use std::path::PathBuf;

use crate::common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::harness::sync::{self, SyncDeps};
use tome::harness::{AgentFormat, HooksStrategy, StubHarness};
use tome::index::{self, OpenOptions};
use tome::mcp::prompts::{self, PromptRegistry};
use tome::mcp::resolve_expose_personas;
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

use crate::common::{stub_embedder_seed, stub_reranker_seed, stub_summariser_seed};

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

fn open_index(paths: &tome::paths::Paths) -> rusqlite::Connection {
    index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
            profile: None,
        },
    )
    .expect("open index db")
}

/// Seed a manifest-less catalog enrolment plus an on-disk plugin
/// `hooks/hooks.json`, returning the catalog URL.
fn seed_hooks_source(paths: &tome::paths::Paths, plugin: &str, body: &str) -> String {
    let url = format!("https://example.test/{plugin}.git");
    let hooks_dir = paths.cache_dir_for(&url).join(plugin).join("hooks");
    std::fs::create_dir_all(&hooks_dir).expect("create hooks source dir");
    std::fs::write(hooks_dir.join("hooks.json"), body).expect("write source hooks.json");
    url
}

fn seed_guardrails_source(paths: &tome::paths::Paths, plugin: &str, body: &str) {
    let url = format!("https://example.test/{plugin}.git");
    let hooks_dir = paths.cache_dir_for(&url).join(plugin).join("hooks");
    std::fs::create_dir_all(&hooks_dir).expect("create hooks source dir");
    std::fs::write(hooks_dir.join("GUARDRAILS.md"), body).expect("write GUARDRAILS.md");
}

fn seed_agent_source(paths: &tome::paths::Paths, plugin: &str, name: &str, body: &str) {
    let url = format!("https://example.test/{plugin}.git");
    let agent_dir = paths.cache_dir_for(&url).join(plugin).join("agents");
    std::fs::create_dir_all(&agent_dir).expect("create agent source dir");
    std::fs::write(agent_dir.join(format!("{name}.md")), body).expect("write source agent");
}

/// Insert an enabled `agent`-kind row for `(catalog, plugin, name)`. An
/// enabled row of any kind also surfaces the plugin in
/// `enabled_plugins_for_workspace`, driving the hooks + guardrails passes too.
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
         VALUES (?1, ?2, ?3, 'agent', 'desc', '1.2.3', ?4, 'h', 0, 0, NULL, '1970-01-01T00:00:00Z')",
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

/// A workspace-startup scope bound to `workspace` with NO project root — the
/// shape the MCP server resolves the persona toggle under. The on-disk
/// workspace `settings.toml` is therefore the declaring layer.
fn workspace_startup_scope(workspace: &WorkspaceName) -> ResolvedScope {
    ResolvedScope {
        scope: Scope(workspace.clone()),
        source: ScopeSource::Flag,
        project_root: None,
        overridden_project_marker: None,
    }
}

#[test]
fn enable_then_sync_lights_up_hooks_guardrails_agents_and_persona() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // One stub harness driving all three sinks: RealJson hooks + a settings
    // path, native agents, and the default in-file guardrails region (no
    // hooks-suppression) on the rules-file target.
    let _guard = HarnessModulesGuard::install(vec![Box::new(
        StubHarness::default()
            .with_hooks_strategy(HooksStrategy::RealJson)
            .with_hook_settings()
            .with_native_agents(AgentFormat::MarkdownYaml),
    )]);

    let fx = Fixture::build("test-workspace", "\"stub\"");

    // Turn the persona surface on at the workspace scope (the layer a
    // project-less startup scope consults). Resolved end-to-end below.
    let ws_settings = fx.paths.workspace_settings_file(&fx.workspace);
    std::fs::create_dir_all(ws_settings.parent().unwrap()).expect("create ws settings dir");
    std::fs::write(
        &ws_settings,
        "name = \"test-workspace\"\nexpose_agents_as_personas = true\n",
    )
    .expect("write workspace settings");

    // One plugin shipping all three component sources. The
    // `${CLAUDE_PLUGIN_ROOT}` token makes the hook rewrite observable; the
    // guardrails body is marker-free (B-1 accepts it); the agent frontmatter
    // carries a `name` we can assert survives translation.
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

    // ----- reconcile every sink in one sync -----
    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    // ===== Surface 1: hooks =====
    // The rewritten hook command resolves `${CLAUDE_PLUGIN_ROOT}` to an
    // ABSOLUTE on-disk path (not the literal token).
    let hooks_path = fx.project.join(".stub/settings.local.json");
    assert!(hooks_path.is_file(), "hook settings file must exist");
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&hooks_path).unwrap()).unwrap();
    let cmd = doc["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .expect("rewritten command string");
    let plugin_root = fx.paths.cache_dir_for(&url).join("plugin-a");
    assert!(
        cmd.starts_with(&*plugin_root.to_string_lossy()),
        "PLUGIN_ROOT resolved to an absolute path; got: {cmd}"
    );
    assert!(
        !cmd.contains("${CLAUDE_PLUGIN_ROOT}"),
        "the literal token must not survive the rewrite; got: {cmd}"
    );

    // ===== Surface 2: guardrails =====
    // The rendered region carries the plugin's GUARDRAILS.md body between the
    // START/END markers (assert the body, not merely the file's existence).
    let guardrails_target = fx.project.join("STUB_RULES.md");
    let rendered = std::fs::read_to_string(&guardrails_target).expect("guardrails target exists");
    let begin = "<!-- START GUARDRAILS: cat:plugin-a -->\n";
    let end = "\n<!-- END GUARDRAILS: cat:plugin-a -->";
    let start_idx = rendered
        .find(begin)
        .unwrap_or_else(|| panic!("START marker present; got:\n{rendered}"))
        + begin.len();
    let end_idx = rendered[start_idx..]
        .find(end)
        .unwrap_or_else(|| panic!("END marker present; got:\n{rendered}"))
        + start_idx;
    let body = &rendered[start_idx..end_idx];
    assert_eq!(
        body, "Be careful with deletes.\n",
        "the rendered region body must equal the GUARDRAILS.md source"
    );

    // ===== Surface 3: native agent =====
    // The agent lands at the `<plugin>__<name>.md` path. The stub's
    // `translate_agent` echoes the frontmatter-STRIPPED body (its deterministic
    // translation, by design — see `src/harness/stub.rs`), so we assert the
    // translated body here. Real MD+YAML frontmatter rendering is proven against
    // the actual claude-code harness in
    // `harness_sync_stub.rs::agent_fans_out_to_multiple_native_harnesses`; this
    // whole-flow proof keeps all three sinks under the single stub.
    let agent_path = fx.project.join(".stub/agents/plugin-a__reviewer.md");
    assert!(
        agent_path.is_file(),
        "native agent file must exist at the <plugin>__<name> path"
    );
    let agent_body = std::fs::read_to_string(&agent_path).expect("read agent file");
    assert!(
        agent_body.contains("You review code."),
        "the agent went through translation (frontmatter split off, body emitted); got:\n{agent_body}"
    );
    assert!(
        !agent_body.contains("description: Reviews code."),
        "the source frontmatter was split off by translation, not copied verbatim; got:\n{agent_body}"
    );

    // ===== Surface 4: persona =====
    // Resolve the toggle from the on-disk workspace setting (NOT a literal),
    // then build the registry. The agent persona + the reserved drop-persona
    // both appear.
    let scope = workspace_startup_scope(&fx.workspace);
    let expose = resolve_expose_personas(&scope, &fx.paths).expect("resolve persona toggle");
    assert!(
        expose,
        "the on-disk workspace setting must resolve the persona toggle to true"
    );

    let conn = open_index(&fx.paths);
    let registry = PromptRegistry::build_for_workspace(&fx.workspace, &fx.paths, &conn, expose)
        .expect("build persona registry");
    let names: Vec<String> = registry.descriptors().into_iter().map(|p| p.name).collect();
    assert!(
        names.contains(&"reviewer-persona".to_owned()),
        "the enabled agent's `<name>-persona` must be exposed; got {names:?}"
    );
    assert!(
        names.contains(&prompts::DROP_PERSONA_NAME.to_owned()),
        "the reserved drop-persona must be exposed; got {names:?}"
    );
}
