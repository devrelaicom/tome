//! Integration tests for `tome harness preview` (issue #288).
//!
//! Drives the read-only preview pipeline (`tome::harness::preview::pipeline`)
//! against the REAL harness modules through a seeded central DB + on-disk plugin
//! source files, so the tests exercise the SAME translation SSOTs
//! (`translate_agent`, the canonical hook enumeration, the tiered-entry query,
//! the guardrails reader) the sync reconcilers use — proving the preview matches
//! what `sync` produces.
//!
//! Coverage per the acceptance criteria:
//!   * native-agent harness (agents native, model field dropped) — `codex`
//!   * native-agent harness that MAPS the model — `opencode`
//!   * rules-only harness (agents → persona / unrepresented) — `cline`
//!   * hook-capable harness (native events) vs GUARDRAILS-fallback — `codex`
//!     (hook_support) vs `opencode` (no hook_support)
//!   * `--plugin` scoping
//!   * unknown harness error (exit 18)
//!   * `--json` shape via the `PreviewReport` serialisation
//!   * nothing-enabled (no DB / empty workspace)

use std::path::PathBuf;

use crate::common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::harness::preview::{self, AgentDelivery, EntryDelivery, PreviewReport};
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

const WS: &str = "test-ws";

struct Fixture {
    _home: TempDir,
    paths: tome::paths::Paths,
    project: PathBuf,
}

impl Fixture {
    fn build() -> Self {
        let env = ToolEnv::new();
        let paths = paths_for(&env);
        std::fs::create_dir_all(&paths.root).expect("create tome root");
        seed_workspace(&paths, WS);

        let project = env.home_path().join("project");
        std::fs::create_dir_all(&project).expect("create project");

        Fixture {
            _home: env.home,
            paths,
            project,
        }
    }

    fn home(&self) -> &std::path::Path {
        self._home.path()
    }

    /// A `ResolvedScope` bound to `WS` with `project_root` set, so the preview
    /// resolves rules/MCP targets like a real project.
    fn scope(&self) -> ResolvedScope {
        ResolvedScope {
            scope: Scope(WorkspaceName::parse(WS).expect("parse ws")),
            source: ScopeSource::ProjectMarker,
            project_root: Some(self.project.clone()),
            overridden_project_marker: None,
        }
    }

    fn preview(&self, harness: &str, plugin: Option<&str>) -> PreviewReport {
        preview::pipeline(harness, plugin, &self.scope(), &self.paths, self.home())
            .expect("preview pipeline")
    }
}

// --- source seeders (manifest-less plugin-root cache) ----------------------

fn plugin_url(plugin: &str) -> String {
    format!("https://example.test/{plugin}.git")
}

fn seed_agent_source(paths: &tome::paths::Paths, plugin: &str, name: &str, body: &str) {
    let dir = paths
        .cache_dir_for(&plugin_url(plugin))
        .join(plugin)
        .join("agents");
    std::fs::create_dir_all(&dir).expect("create agent source dir");
    std::fs::write(dir.join(format!("{name}.md")), body).expect("write source agent");
}

fn seed_hooks_source(paths: &tome::paths::Paths, plugin: &str, body: &str) {
    let dir = paths
        .cache_dir_for(&plugin_url(plugin))
        .join(plugin)
        .join("hooks");
    std::fs::create_dir_all(&dir).expect("create hooks source dir");
    std::fs::write(dir.join("hooks.json"), body).expect("write hooks.json");
}

fn seed_guardrails_source(paths: &tome::paths::Paths, plugin: &str, body: &str) {
    let dir = paths
        .cache_dir_for(&plugin_url(plugin))
        .join(plugin)
        .join("hooks");
    std::fs::create_dir_all(&dir).expect("create hooks source dir");
    std::fs::write(dir.join("GUARDRAILS.md"), body).expect("write GUARDRAILS.md");
}

fn enrol_catalog(paths: &tome::paths::Paths, catalog: &str, plugin: &str) {
    let conn = rusqlite::Connection::open(&paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, WS, catalog, &plugin_url(plugin), "main")
        .expect("enrol catalog");
}

fn insert_enabled_row(
    paths: &tome::paths::Paths,
    catalog: &str,
    plugin: &str,
    name: &str,
    kind: &str,
    path: &str,
) {
    let conn = rusqlite::Connection::open(&paths.index_db).expect("open rw");
    conn.execute(
        "INSERT INTO skills
            (catalog, plugin, name, kind, description, plugin_version,
             path, content_hash, searchable, user_invocable, when_to_use, indexed_at)
         VALUES (?1, ?2, ?3, ?4, 'd', '0.0.0', ?5, 'h', 1, 0, NULL, '1970-01-01T00:00:00Z')",
        rusqlite::params![catalog, plugin, name, kind, path],
    )
    .expect("insert row");
    let skill_id: i64 = conn
        .query_row(
            "SELECT id FROM skills WHERE catalog=?1 AND plugin=?2 AND kind=?3 AND name=?4",
            rusqlite::params![catalog, plugin, kind, name],
            |r| r.get(0),
        )
        .expect("skill id");
    let ws_id: i64 = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name = ?1",
            rusqlite::params![WS],
            |r| r.get(0),
        )
        .expect("ws id");
    conn.execute(
        "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at) VALUES (?1, ?2, 0)",
        rusqlite::params![ws_id, skill_id],
    )
    .expect("enrol");
}

fn insert_enabled_agent(paths: &tome::paths::Paths, catalog: &str, plugin: &str, name: &str) {
    insert_enabled_row(
        paths,
        catalog,
        plugin,
        name,
        "agent",
        &format!("agents/{name}.md"),
    );
}

fn insert_enabled_skill(paths: &tome::paths::Paths, catalog: &str, plugin: &str, name: &str) {
    insert_enabled_row(
        paths,
        catalog,
        plugin,
        name,
        "skill",
        &format!("skills/{name}/SKILL.md"),
    );
}

fn insert_enabled_command(paths: &tome::paths::Paths, catalog: &str, plugin: &str, name: &str) {
    insert_enabled_row(
        paths,
        catalog,
        plugin,
        name,
        "command",
        &format!("commands/{name}.md"),
    );
}

/// Seed one plugin with a model-pinned agent + a skill + a command + a
/// PreToolUse hook + GUARDRAILS.md, all enabled in `WS`.
fn seed_full_plugin(paths: &tome::paths::Paths, catalog: &str, plugin: &str) {
    seed_agent_source(
        paths,
        plugin,
        "reviewer",
        "---\nname: reviewer\ndescription: Reviews code.\nmodel: opus\ntools: [Read, Grep]\n---\nYou review code.\n",
    );
    seed_hooks_source(
        paths,
        plugin,
        r#"{ "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/guard.sh" } ] } ] }"#,
    );
    seed_guardrails_source(paths, plugin, "Be careful with deletes.\n");
    enrol_catalog(paths, catalog, plugin);
    insert_enabled_agent(paths, catalog, plugin, "reviewer");
    insert_enabled_skill(paths, catalog, plugin, "my-skill");
    insert_enabled_command(paths, catalog, plugin, "do-thing");
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------------------
// Native-agent harness: agents translated natively; model dropped (codex).
// ---------------------------------------------------------------------------

#[test]
fn codex_native_agent_drops_model_field() {
    let _lock = lock();
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::codex::CODEX)]);
    let fx = Fixture::build();
    seed_full_plugin(&fx.paths, "cat", "plug");

    let report = fx.preview("codex", None);

    assert!(report.supports_native_agents, "codex emits native agents");
    assert_eq!(report.agents.len(), 1, "one enabled agent");
    let a = &report.agents[0];
    assert_eq!(a.name, "reviewer");
    match &a.delivery {
        AgentDelivery::Native {
            filename,
            dropped_fields,
            ..
        } => {
            // Filename provenance is `<plugin>__<name>.<ext>`; codex uses TOML.
            assert!(
                filename.starts_with("plug__reviewer"),
                "filename provenance: {filename}"
            );
            // Codex's map_model returns None for `opus` (no OpenAI target) → the
            // model field drops. This is the exact `translate_agent` drop list.
            assert!(
                dropped_fields.iter().any(|f| f == "model"),
                "codex must drop the model field; got {dropped_fields:?}"
            );
        }
        other => panic!("expected Native delivery, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Native-agent harness that MAPS the model (opencode: opus → anthropic/<id>).
// opencode has NO hook_support → hooks fall back to GUARDRAILS.
// ---------------------------------------------------------------------------

#[test]
fn opencode_native_agent_and_guardrails_hook_fallback() {
    let _lock = lock();
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::opencode::OPENCODE)]);
    let fx = Fixture::build();
    seed_full_plugin(&fx.paths, "cat", "plug");

    let report = fx.preview("opencode", None);

    assert!(report.supports_native_agents);
    assert!(
        !report.supports_native_hooks,
        "opencode has no native hook translation"
    );
    let a = &report.agents[0];
    match &a.delivery {
        AgentDelivery::Native { dropped_fields, .. } => {
            // opencode resolves `opus` → `anthropic/<id>` via the registry, so
            // the model is NOT dropped (unlike codex). This proves the preview
            // uses the harness's OWN translate_agent, not an approximation.
            assert!(
                !dropped_fields.iter().any(|f| f == "model"),
                "opencode maps opus, so model must NOT drop; got {dropped_fields:?}"
            );
        }
        other => panic!("expected Native, got {other:?}"),
    }

    // Hooks: the PreToolUse hook has no native target on opencode → it falls
    // back to GUARDRAILS, and the plugin ships GUARDRAILS.md prose.
    assert_eq!(report.hooks.len(), 1);
    let h = &report.hooks[0];
    assert!(
        h.native_events.is_empty(),
        "opencode translates no events natively"
    );
    assert!(
        h.guardrails_events.iter().any(|e| e == "PreToolUse"),
        "PreToolUse must fall back to GUARDRAILS; got {:?}",
        h.guardrails_events
    );
    assert!(h.has_guardrails_prose, "plugin ships GUARDRAILS.md");
}

// ---------------------------------------------------------------------------
// Hook-capable harness: PreToolUse reaches codex NATIVELY (#318).
// ---------------------------------------------------------------------------

#[test]
fn codex_translates_pretooluse_natively() {
    let _lock = lock();
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::codex::CODEX)]);
    let fx = Fixture::build();
    seed_full_plugin(&fx.paths, "cat", "plug");

    let report = fx.preview("codex", None);

    assert!(
        report.supports_native_hooks,
        "codex supports native hook translation"
    );
    assert_eq!(report.hooks.len(), 1);
    let h = &report.hooks[0];
    assert!(
        h.native_events.iter().any(|e| e == "PreToolUse"),
        "PreToolUse must reach codex natively; got native={:?} guardrails={:?}",
        h.native_events,
        h.guardrails_events
    );
    assert!(
        !h.guardrails_events.iter().any(|e| e == "PreToolUse"),
        "PreToolUse must NOT also be a GUARDRAILS fallback on codex"
    );
}

// ---------------------------------------------------------------------------
// R3: mixed events — a plugin with a supported (PreToolUse) AND an unsupported
// (SessionEnd) event on codex → native PreToolUse, guardrails SessionEnd. This
// exercises the declared-but-unsupported-event → guardrails branch at the
// PIPELINE level (codex supports PreToolUse but not SessionEnd).
// ---------------------------------------------------------------------------

#[test]
fn codex_mixed_supported_and_unsupported_events_split_correctly() {
    let _lock = lock();
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::codex::CODEX)]);
    let fx = Fixture::build();
    // A command-handler hooks.json with BOTH PreToolUse (codex supports) and
    // SessionEnd (codex does NOT support → GUARDRAILS).
    seed_hooks_source(
        &fx.paths,
        "plug",
        r#"{
            "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/pre.sh" } ] } ],
            "SessionEnd": [ { "hooks": [ { "type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/end.sh" } ] } ]
        }"#,
    );
    enrol_catalog(&fx.paths, "cat", "plug");
    insert_enabled_skill(&fx.paths, "cat", "plug", "s");

    let report = fx.preview("codex", None);

    assert_eq!(report.hooks.len(), 1);
    let h = &report.hooks[0];
    assert_eq!(
        h.native_events,
        vec!["PreToolUse".to_string()],
        "only PreToolUse is native on codex; got {h:?}"
    );
    assert_eq!(
        h.guardrails_events,
        vec!["SessionEnd".to_string()],
        "SessionEnd (unsupported by codex) falls back to GUARDRAILS; got {h:?}"
    );
}

// ---------------------------------------------------------------------------
// IMPORTANT (#1): a prompt-ONLY hook is dropped to GUARDRAILS by sync under the
// DEFAULT config (prompts disabled), and only reaches the harness natively when
// prompt_provider/prompt_model is configured. The preview must mirror this gate.
// ---------------------------------------------------------------------------

/// A prompt-only PreToolUse hook, previewed against codex (which supports
/// PreToolUse). With prompts DISABLED (default), sync filters the prompt handler
/// before computing `used`, so the event drops to GUARDRAILS — the preview must
/// report it in `guardrails_events`, not `native_events`.
#[test]
fn prompt_only_hook_is_guardrails_by_default() {
    let _lock = lock();
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::codex::CODEX)]);
    let fx = Fixture::build();
    seed_hooks_source(
        &fx.paths,
        "plug",
        r#"{ "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "prompt", "prompt": "Is this safe?" } ] } ] }"#,
    );
    enrol_catalog(&fx.paths, "cat", "plug");
    insert_enabled_skill(&fx.paths, "cat", "plug", "s");

    let report = fx.preview("codex", None);

    assert_eq!(report.hooks.len(), 1);
    let h = &report.hooks[0];
    assert!(
        h.native_events.is_empty(),
        "a prompt-only hook must NOT be native when prompts are disabled (matches sync); got {h:?}"
    );
    assert_eq!(
        h.guardrails_events,
        vec!["PreToolUse".to_string()],
        "a prompt-only PreToolUse must fall back to GUARDRAILS by default; got {h:?}"
    );
}

/// The SAME prompt-only hook, but with a prompt provider + model configured, is
/// translated natively — matching sync's `effective_canonical` (prompts kept).
#[test]
fn prompt_only_hook_is_native_when_prompt_model_configured() {
    let _lock = lock();
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::codex::CODEX)]);
    let fx = Fixture::build();
    // Configure a prompt provider + model in the global config so the gate opens.
    std::fs::write(
        fx.paths.root.join("config.toml"),
        "[hooks]\nprompt_provider = \"openai\"\nprompt_model = \"gpt-4o-mini\"\n",
    )
    .expect("write config");
    seed_hooks_source(
        &fx.paths,
        "plug",
        r#"{ "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "prompt", "prompt": "Is this safe?" } ] } ] }"#,
    );
    enrol_catalog(&fx.paths, "cat", "plug");
    insert_enabled_skill(&fx.paths, "cat", "plug", "s");

    let report = fx.preview("codex", None);

    assert_eq!(report.hooks.len(), 1);
    let h = &report.hooks[0];
    assert_eq!(
        h.native_events,
        vec!["PreToolUse".to_string()],
        "with a prompt model configured, the prompt hook reaches codex natively; got {h:?}"
    );
    assert!(
        h.guardrails_events.is_empty(),
        "no GUARDRAILS fallback once the prompt gate is open; got {h:?}"
    );
}

// ---------------------------------------------------------------------------
// R1: a malformed hooks.json is SURFACED as a report-level note (hooks_error),
// not silently omitted (consistent with the agent path's per-entry errors).
// ---------------------------------------------------------------------------

#[test]
fn malformed_hooks_json_is_surfaced_as_report_error() {
    let _lock = lock();
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::codex::CODEX)]);
    let fx = Fixture::build();
    // An unparsable hooks.json → resolve_enabled_canonical_hooks records a
    // first_error and omits the plugin's hooks; the preview surfaces the error.
    seed_hooks_source(&fx.paths, "plug", "{ this is not valid json ");
    enrol_catalog(&fx.paths, "cat", "plug");
    insert_enabled_skill(&fx.paths, "cat", "plug", "s");

    let report = fx.preview("codex", None);

    assert!(
        report.hooks_error.is_some(),
        "a malformed hooks.json must surface a report-level hooks_error, not be silently dropped"
    );
}

// ---------------------------------------------------------------------------
// Rules-only harness: agents → persona (personas on) / unrepresented (off).
// ---------------------------------------------------------------------------

#[test]
fn cline_rules_only_agents_unrepresented_by_default() {
    let _lock = lock();
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::cline::CLINE)]);
    let fx = Fixture::build();
    seed_full_plugin(&fx.paths, "cat", "plug");

    let report = fx.preview("cline", None);

    assert!(
        !report.supports_native_agents,
        "cline is a rules-only harness"
    );
    assert!(!report.personas_enabled, "personas default off");
    assert_eq!(report.agents.len(), 1);
    assert_eq!(
        report.agents[0].delivery,
        AgentDelivery::Unrepresented,
        "rules-only harness with personas off → unrepresented"
    );
}

#[test]
fn cline_rules_only_agents_become_personas_when_enabled() {
    let _lock = lock();
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::cline::CLINE)]);
    let fx = Fixture::build();
    // Turn personas on in the global config.
    std::fs::write(
        fx.paths.root.join("config.toml"),
        "[harness]\nexpose_agents_as_personas = true\n",
    )
    .expect("write config");
    seed_full_plugin(&fx.paths, "cat", "plug");

    let report = fx.preview("cline", None);

    assert!(report.personas_enabled);
    assert_eq!(
        report.agents[0].delivery,
        AgentDelivery::Persona,
        "rules-only harness with personas on → MCP persona"
    );
}

// ---------------------------------------------------------------------------
// Skills / commands are always MCP-routed (get_skill / MCP prompt).
// ---------------------------------------------------------------------------

#[test]
fn skills_and_commands_are_mcp_routed() {
    let _lock = lock();
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::codex::CODEX)]);
    let fx = Fixture::build();
    seed_full_plugin(&fx.paths, "cat", "plug");

    let report = fx.preview("codex", None);

    // Exactly the skill + command (the agent is NOT in entries).
    assert_eq!(report.entries.len(), 2, "one skill + one command");
    let skill = report
        .entries
        .iter()
        .find(|e| e.kind == "skill")
        .expect("skill entry");
    assert_eq!(skill.delivery, EntryDelivery::McpGetSkill);
    let cmd = report
        .entries
        .iter()
        .find(|e| e.kind == "command")
        .expect("command entry");
    assert_eq!(cmd.delivery, EntryDelivery::McpPrompt);
}

// ---------------------------------------------------------------------------
// --plugin scoping.
// ---------------------------------------------------------------------------

#[test]
fn plugin_filter_scopes_every_section() {
    let _lock = lock();
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::codex::CODEX)]);
    let fx = Fixture::build();
    // Distinct catalogs per plugin — the catalog enrolment is keyed on catalog
    // name, so two plugins must live under two catalogs.
    seed_full_plugin(&fx.paths, "cat-keep", "keep");
    seed_full_plugin(&fx.paths, "cat-drop", "drop");

    // Without a filter: both plugins' entries appear.
    let all = fx.preview("codex", None);
    assert_eq!(all.agents.len(), 2, "both plugins' agents");
    assert_eq!(all.entries.len(), 4, "2 plugins × (skill + command)");
    assert_eq!(all.hooks.len(), 2, "both plugins' hooks");

    // With --plugin keep: only `keep`.
    let scoped = fx.preview("codex", Some("keep"));
    assert!(scoped.agents.iter().all(|a| a.plugin == "keep"));
    assert_eq!(scoped.agents.len(), 1);
    assert!(scoped.entries.iter().all(|e| e.plugin == "keep"));
    assert_eq!(scoped.entries.len(), 2);
    assert!(scoped.hooks.iter().all(|h| h.plugin == "keep"));
    assert_eq!(scoped.hooks.len(), 1);
    assert_eq!(scoped.plugin_filter.as_deref(), Some("keep"));
}

// ---------------------------------------------------------------------------
// Unknown harness → HarnessNotSupported (exit 18).
// ---------------------------------------------------------------------------

#[test]
fn unknown_harness_errors_with_exit_18() {
    let _lock = lock();
    let fx = Fixture::build();
    let err = preview::pipeline(
        "definitely-not-a-harness",
        None,
        &fx.scope(),
        &fx.paths,
        fx.home(),
    )
    .expect_err("unknown harness must error");
    assert_eq!(
        err.exit_code(),
        18,
        "HarnessNotSupported → exit 18; got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Nothing enabled: empty workspace + no DB both produce empty (not an error).
// ---------------------------------------------------------------------------

#[test]
fn empty_workspace_produces_empty_report() {
    let _lock = lock();
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::codex::CODEX)]);
    let fx = Fixture::build();
    // Workspace seeded but nothing enabled.
    let report = fx.preview("codex", None);
    assert!(report.agents.is_empty());
    assert!(report.entries.is_empty());
    assert!(report.hooks.is_empty());
    assert_eq!(report.workspace, WS);
    // The harness metadata still resolves.
    assert!(report.supports_native_agents);
}

#[test]
fn no_index_db_produces_empty_report_not_an_error() {
    let _lock = lock();
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::codex::CODEX)]);
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).expect("create tome root");
    // Deliberately do NOT bootstrap the index DB.
    assert!(!paths.index_db.exists());
    let scope = ResolvedScope {
        scope: Scope(WorkspaceName::parse(WS).expect("ws")),
        source: ScopeSource::ProjectMarker,
        project_root: Some(env.home_path().join("project")),
        overridden_project_marker: None,
    };
    let report = preview::pipeline("codex", None, &scope, &paths, env.home_path())
        .expect("no-DB preview must succeed (empty)");
    assert!(report.agents.is_empty());
    assert!(report.entries.is_empty());
    assert!(report.hooks.is_empty());
}

// ---------------------------------------------------------------------------
// --json shape: the PreviewReport serialises to a stable structured document.
// ---------------------------------------------------------------------------

#[test]
fn json_report_shape_is_stable() {
    let _lock = lock();
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::codex::CODEX)]);
    let fx = Fixture::build();
    seed_full_plugin(&fx.paths, "cat", "plug");

    let report = fx.preview("codex", Some("plug"));
    let v: serde_json::Value = serde_json::to_value(&report).expect("serialise");

    // Top-level keys.
    assert_eq!(v["harness"], "codex");
    assert_eq!(v["workspace"], WS);
    assert_eq!(v["plugin_filter"], "plug");
    assert_eq!(v["supports_native_agents"], true);
    assert_eq!(v["supports_native_hooks"], true);
    assert_eq!(v["mcp_manual_only"], false);

    // Agents: the delivery tag is flattened onto each agent record.
    let agent = &v["agents"][0];
    assert_eq!(agent["delivery"], "native");
    assert!(
        agent["filename"]
            .as_str()
            .unwrap()
            .starts_with("plug__reviewer")
    );
    assert!(
        agent["dropped_fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "model")
    );

    // Entries: kind + delivery.
    let entries = v["entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .any(|e| e["kind"] == "command" && e["delivery"] == "mcp_prompt")
    );
    assert!(
        entries
            .iter()
            .any(|e| e["kind"] == "skill" && e["delivery"] == "mcp_get_skill")
    );

    // Hooks: native_events + guardrails_events + has_guardrails_prose.
    let hook = &v["hooks"][0];
    assert!(
        hook["native_events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e == "PreToolUse")
    );
    assert_eq!(hook["has_guardrails_prose"], true);
}

// ---------------------------------------------------------------------------
// Issue #292: the doctor unrepresented-hooks report agrees with the preview's
// GUARDRAILS-fallback events on a rules-only-for-hooks harness (the SSOT reuse
// + SAME-SET-SAME-RESOLVER guarantee — both call the same canonical-hook
// enumeration + `hook_support()`/rules-only definition).
// ---------------------------------------------------------------------------

#[test]
fn doctor_unrepresented_hooks_agrees_with_preview_guardrails_events() {
    use tome::doctor::checks::build_unrepresented_hooks_report;
    use tome::settings::resolver::{EffectiveHarness, EffectiveHarnessList};

    let _lock = lock();
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::cline::CLINE)]);
    let fx = Fixture::build();
    seed_full_plugin(&fx.paths, "cat", "plug");

    // Preview side: cline is rules-only for hooks, so the plugin's PreToolUse
    // hook falls to GUARDRAILS (never native).
    let preview = fx.preview("cline", None);
    assert!(
        !preview.supports_native_hooks,
        "cline has no native hook translation",
    );
    let preview_guardrails: Vec<String> = preview
        .hooks
        .iter()
        .flat_map(|h| h.guardrails_events.clone())
        .collect();
    assert!(
        preview_guardrails.contains(&"PreToolUse".to_string()),
        "preview must report PreToolUse as a GUARDRAILS-fallback event: {preview_guardrails:?}",
    );

    // Doctor side: the SAME (harness × enabled plugin hooks) set via the shared
    // SSOT, scoped to cline.
    let effective = EffectiveHarnessList {
        harnesses: vec![EffectiveHarness {
            name: "cline".to_owned(),
            source_chain: vec!["project".to_owned()],
        }],
        excluded: vec![],
    };
    let ws = WorkspaceName::parse(WS).expect("parse ws");
    let doctor = build_unrepresented_hooks_report(&fx.paths, &ws, fx.home(), Some(&effective))
        .expect("build_unrepresented_hooks_report");

    assert_eq!(
        doctor.rules_only_harnesses,
        vec!["cline".to_owned()],
        "cline must be the rules-only-for-hooks harness",
    );
    let doctor_events: Vec<String> = doctor.hooks.iter().map(|h| h.event.clone()).collect();
    assert!(
        doctor_events.contains(&"PreToolUse".to_string()),
        "doctor must report the same PreToolUse event preview flags as GUARDRAILS: {doctor_events:?}",
    );
    // Agreement: every event doctor reports unrepresented is a GUARDRAILS event
    // in the preview (same native-vs-guardrails decision, single SSOT).
    for ev in &doctor_events {
        assert!(
            preview_guardrails.contains(ev),
            "doctor event {ev} not in preview guardrails_events {preview_guardrails:?}",
        );
    }
}

/// A hook-capable harness (`codex`) delivers the plugin's supported events
/// natively, so the doctor unrepresented-hooks report is EMPTY for it — proving
/// the report excludes the five `#318` dispatcher harnesses (not just the
/// unit-tested exclusion, but through the seeded canonical-hook enumeration).
#[test]
fn doctor_unrepresented_hooks_empty_for_hook_capable_harness() {
    use tome::doctor::checks::build_unrepresented_hooks_report;
    use tome::settings::resolver::{EffectiveHarness, EffectiveHarnessList};

    let _lock = lock();
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::codex::CODEX)]);
    let fx = Fixture::build();
    seed_full_plugin(&fx.paths, "cat", "plug");

    let effective = EffectiveHarnessList {
        harnesses: vec![EffectiveHarness {
            name: "codex".to_owned(),
            source_chain: vec!["project".to_owned()],
        }],
        excluded: vec![],
    };
    let ws = WorkspaceName::parse(WS).expect("parse ws");
    let doctor = build_unrepresented_hooks_report(&fx.paths, &ws, fx.home(), Some(&effective))
        .expect("build_unrepresented_hooks_report");

    assert!(
        doctor.rules_only_harnesses.is_empty(),
        "codex is hook-capable → not rules-only for hooks: {:?}",
        doctor.rules_only_harnesses,
    );
    assert!(
        doctor.hooks.is_empty(),
        "no rules-only-for-hooks harness in scope → no unrepresented hooks: {:?}",
        doctor.hooks,
    );
}
