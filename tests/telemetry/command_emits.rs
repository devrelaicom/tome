//! Per-command anonymous emits, asserted at the produced kernel queue line.
//!
//! Re-homed onto the `gauge-telemetry` kernel: the produced queue line is the
//! kernel `QueuedEvent` (`{"event_name":...,"attributes":{...}}`), so the event
//! name is read from `event_name` and the dimensions from `attributes`. Emits no
//! longer carry a per-line install/session uuid (the kernel attaches identity as
//! OTLP resource attributes at drain time only), and quantities are RAW integers
//! (the kernel buckets at read time) — `latency_ms`/`candidates_returned`/
//! `corpus_size`/`findings`/`rank`, never the old bucket-token strings.
//!
//! Two driving models live here, each used where it is the faithful one:
//!
//! 1. **Real binary** (`ToolEnv` + `Command`): the catalog / workspace / doctor
//!    command paths, whose telemetry emits live in the CLI command WRAPPERS and
//!    which do NOT load ONNX. Each spawned `Command` clears every CI /
//!    `TOME_TELEMETRY*` var then force-enables telemetry (`TOME_TELEMETRY=1`,
//!    overriding CI auto-off) with a loopback endpoint so the kernel `build()`
//!    validates without ever connecting (emit only appends).
//! 2. **In-process, stub-embedder** (`query::run_with_deps`): the CLI
//!    `tome.search { surface: cli }` path. The real `tome query` binary loads
//!    real ONNX models, so the only stub-only way to reach the CLI search emit is
//!    its `pub` library entry — which routes through the SAME process-global
//!    `telemetry::emit` the binary does. The staged tree is rooted at
//!    `$HOME/.tome` (a `HomeGuard` pins it) and telemetry is force-enabled, so the
//!    handler's process-global emit lands where the test reads it.

use std::process::Command;

use crate::common::ToolEnv;
use crate::queue_util::{
    LOOPBACK_ENDPOINT, TELEMETRY_ENV_VARS, attr, count_named, first_named, queue_events_in_root,
};

/// A `tome` command over the isolated `$HOME`, every CI/telemetry var removed,
/// then telemetry FORCE-ENABLED (`TOME_TELEMETRY=1`, overriding CI auto-off) and
/// pointed at a loopback endpoint the kernel `build()` accepts (emit only
/// appends, so nothing ever connects).
fn force_on_cmd(env: &ToolEnv) -> Command {
    let mut cmd = env.cmd();
    for &k in TELEMETRY_ENV_VARS {
        cmd.env_remove(k);
    }
    cmd.env("TOME_TELEMETRY", "1");
    cmd.env("TOME_GAUGE_ENDPOINT", LOOPBACK_ENDPOINT);
    cmd
}

/// Read the queued telemetry lines under the isolated `$HOME/.tome`.
fn queue_events(env: &ToolEnv) -> Vec<serde_json::Value> {
    queue_events_in_root(&env.tome_root())
}

// ===========================================================================
// catalog_action (local) + the success-gate on a failed add.
// ===========================================================================

/// `catalog add <file:// fixture>` ⇒ a `tome.catalog_action { action: added,
/// source_type: local }`. The resolved `file://` URL drives the `Local` branch.
#[test]
fn catalog_add_local_emits_catalog_action_added_local() {
    use crate::common::Fixture;

    let fix = Fixture::build_sample();
    let env = ToolEnv::new();

    let out = force_on_cmd(&env)
        .args(["catalog", "add", &fix.url])
        .output()
        .expect("spawn tome");
    assert!(
        out.status.success(),
        "catalog add exited {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    let events = queue_events(&env);
    let ev = first_named(&events, "tome.catalog_action")
        .unwrap_or_else(|| panic!("no tome.catalog_action in queue: {events:?}"));
    assert_eq!(
        attr(ev, "action"),
        "added",
        "catalog add ⇒ action=added: {ev}"
    );
    assert_eq!(
        attr(ev, "source_type"),
        "local",
        "a file:// source resolves to source_type=local: {ev}"
    );
}

/// A FAILED `catalog add` (a git URL pointing at nothing → exit 6) emits NO
/// `tome.catalog_action` — the emit is gated on a successful add.
#[test]
fn failed_catalog_add_emits_no_catalog_action() {
    let env = ToolEnv::new();
    // A credential-bearing git URL pointing at nothing — git fails (exit 6)
    // before the success-gated emit is ever reached.
    let bad_url = "https://alice:supersecret@127.0.0.1:1/nope.git";

    let out = force_on_cmd(&env)
        .args(["catalog", "add", bad_url])
        .output()
        .expect("spawn tome");
    assert_eq!(
        out.status.code(),
        Some(6),
        "a bad git URL must fail the clone (exit 6); stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    let events = queue_events(&env);
    assert_eq!(
        count_named(&events, "tome.catalog_action"),
        0,
        "a failed catalog add must emit no catalog_action (success-gated): {events:?}",
    );
}

// ===========================================================================
// workspace_action: init emits; a no-op `use` does NOT (success gate).
// ===========================================================================

/// `workspace init <name>` ⇒ a `tome.workspace_action { action: init }`.
#[test]
fn workspace_init_emits_workspace_action_init() {
    let env = ToolEnv::new();

    let out = force_on_cmd(&env)
        .args(["workspace", "init", "proj-x"])
        .output()
        .expect("spawn tome");
    assert!(
        out.status.success(),
        "workspace init exited {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    let events = queue_events(&env);
    let ev = first_named(&events, "tome.workspace_action")
        .unwrap_or_else(|| panic!("no tome.workspace_action in queue: {events:?}"));
    assert_eq!(
        attr(ev, "action"),
        "init",
        "workspace init ⇒ action=init: {ev}"
    );
}

/// A no-op `workspace use <nonexistent>` (exit 13, WorkspaceNotFound) emits NO
/// `tome.workspace_action` — proving the success-gate.
#[test]
fn noop_workspace_use_emits_no_workspace_action() {
    let env = ToolEnv::new();
    let cwd = tempfile::TempDir::new().unwrap();

    let out = force_on_cmd(&env)
        .current_dir(cwd.path())
        .args(["workspace", "use", "does-not-exist"])
        .output()
        .expect("spawn tome");
    assert_eq!(
        out.status.code(),
        Some(13),
        "use of a nonexistent workspace must be WorkspaceNotFound (exit 13); stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    let events = queue_events(&env);
    assert_eq!(
        count_named(&events, "tome.workspace_action"),
        0,
        "a failed `workspace use` must emit nothing (success gate): {events:?}",
    );
}

// ===========================================================================
// doctor_run: a `tome doctor` invocation emits the run event.
// ===========================================================================

/// `tome doctor` ⇒ a `tome.doctor_run { fix: false, findings: <int> }`.
/// `doctor` may classify a fresh home as degraded (exit 1), but the emit fires
/// BEFORE the exit path, so it lands regardless of the exit code.
#[test]
fn doctor_emits_doctor_run_with_fix_flag_and_findings() {
    let env = ToolEnv::new();

    let out = force_on_cmd(&env)
        .args(["doctor"])
        .output()
        .expect("spawn tome");
    let code = out.status.code();
    assert!(
        code == Some(0) || code == Some(1),
        "doctor should exit 0 or 1 (degraded), got {code:?}; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    let events = queue_events(&env);
    let ev = first_named(&events, "tome.doctor_run")
        .unwrap_or_else(|| panic!("no tome.doctor_run in queue: {events:?}"));
    assert_eq!(
        attr(ev, "fix"),
        &serde_json::Value::Bool(false),
        "doctor (no --fix) ⇒ fix=false: {ev}"
    );
    assert!(
        attr(ev, "findings").is_number(),
        "doctor_run carries a raw integer findings count: {ev}"
    );
}

/// `tome doctor --fix` ⇒ a `tome.doctor_run { fix: true, .. }`.
#[test]
fn doctor_fix_emits_doctor_run_with_fix_true() {
    let env = ToolEnv::new();

    let out = force_on_cmd(&env)
        .args(["doctor", "--fix"])
        .output()
        .expect("spawn tome");
    let code = out.status.code();
    assert!(
        matches!(code, Some(0) | Some(1) | Some(75)),
        "doctor --fix exit {code:?}; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    let events = queue_events(&env);
    let ev = first_named(&events, "tome.doctor_run")
        .unwrap_or_else(|| panic!("no tome.doctor_run in queue: {events:?}"));
    assert_eq!(
        attr(ev, "fix"),
        &serde_json::Value::Bool(true),
        "doctor --fix ⇒ fix=true: {ev}"
    );
}

// ===========================================================================
// plugin_action: the process-global `emit` round-trip (stub-only).
//
// The enable/disable CLI wrappers load real ONNX with no stub seam, so the
// enable/disable emit is NOT reachable end-to-end with stubs via the binary.
// This asserts the EXACT events those wrappers emit (`PluginActionEvent` with
// `Enabled` / `Disabled`) round-trip through the process-global `telemetry::emit`
// the wrappers depend on, which the binary path can't exercise in fast CI.
// ===========================================================================

#[test]
fn plugin_action_emit_round_trips_enabled_and_disabled() {
    use crate::common::HomeGuard;
    use crate::queue_util::queue_events;
    use tome::paths::Paths;
    use tome::telemetry::event::{PluginAction, PluginActionEvent};

    let home = tempfile::TempDir::new().unwrap();
    let _home_guard = HomeGuard::install(home.path());
    let _env = EnvForce::install();

    let paths = Paths::from_root(home.path().join(".tome"));
    // Point the process-global emit path at this isolated queue. `init`'s global
    // is set-once, so an in-process emit test installs an ENABLED handle (built
    // from the forced-on env + loopback endpoint) into the test override slot.
    let _handle = tome::telemetry::TelemetryHandleGuard::install(
        tome::telemetry::build_handle_for_test(&paths),
    );

    // Both variants through the process-global `emit` (the exact call the CLI
    // wrappers make).
    tome::telemetry::emit(PluginActionEvent {
        action: PluginAction::Enabled,
    });
    tome::telemetry::emit(PluginActionEvent {
        action: PluginAction::Disabled,
    });

    let events = queue_events(&paths);
    let actions: Vec<&str> = events
        .iter()
        .filter(|e| e["event_name"] == "tome.plugin_action")
        .map(|e| e["attributes"]["action"].as_str().expect("action string"))
        .collect();
    assert_eq!(
        actions,
        vec!["enabled", "disabled"],
        "both plugin_action variants land with the right action tokens: {events:?}",
    );
}

// ===========================================================================
// CLI `tome.search { surface: cli }` end-to-end via the library entry.
//
// Unix-only: the staging symlinks the catalog cache dir (the same shape
// `mcp_funnel.rs` uses), so this section is gated like its peer.
// ===========================================================================

#[cfg(unix)]
mod cli_search {
    use super::*;

    use std::path::Path;

    use serde_json::Value;
    use tempfile::TempDir;
    use tome::cli::QueryArgs;
    use tome::commands::query;
    use tome::config::Config;
    use tome::embedding::stub::{StubEmbedder, StubReranker};
    use tome::index::{self, OpenOptions};
    use tome::output::Mode;
    use tome::paths::Paths;
    use tome::plugin::PluginId;
    use tome::plugin::lifecycle::{self, LifecycleDeps};
    use tome::workspace::{Scope, WorkspaceName};

    use crate::common::{
        HomeGuard, config_with_catalog, fabricate_models, stub_embedder_seed, stub_reranker_seed,
        stub_summariser_seed,
    };
    use crate::queue_util::queue_events;

    /// The pinned registry embedder name — the value `Search.embedder_model_id`
    /// carries (the DEFAULT profile's pinned embedder).
    fn registry_embedder_name() -> &'static str {
        tome::embedding::profile::embedder_for(tome::embedding::profile::Profile::DEFAULT).name
    }

    fn open_index(paths: &Paths) -> rusqlite::Connection {
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

    fn seed_catalog_enrolment(paths: &Paths, catalog_root: &Path, catalog_name: &str) {
        let url = format!("file://{}", catalog_root.display());
        let conn = open_index(paths);
        tome::index::workspace_catalogs::insert(&conn, "global", catalog_name, &url, "main")
            .expect("seed workspace_catalogs");
        drop(conn);

        let cache_dir = paths.cache_dir_for(&url);
        if let Some(parent) = cache_dir.parent() {
            std::fs::create_dir_all(parent).expect("create catalogs parent");
        }
        if !cache_dir.exists() {
            std::os::unix::fs::symlink(catalog_root, &cache_dir).expect("symlink catalog cache");
        }
    }

    fn skill_body(name: &str, description: &str) -> String {
        format!(
            "---\nname: {name}\ndescription: {description}\n---\n# {name}\n\nBody for {name}.\n"
        )
    }

    /// Stage `acme/plug` rooted at `home/.tome`, enabled + indexed against
    /// `global` with the StubEmbedder. The process-global telemetry handle is
    /// built (enabled, loopback endpoint) so the search emit appends.
    fn stage_at_home(home: &Path, skills: &[(&str, &str)]) -> Paths {
        let root = home.join(".tome");
        let paths = Paths::from_root(root.clone());
        std::fs::create_dir_all(&paths.root).unwrap();
        fabricate_models(&paths);

        let catalog_root = home.join("catalog");
        std::fs::create_dir_all(&catalog_root).unwrap();
        let config = config_with_catalog("acme", &catalog_root);

        let plugin_dir = catalog_root.join("plug");
        std::fs::create_dir_all(plugin_dir.join(".claude-plugin")).unwrap();
        std::fs::write(
            plugin_dir.join("tome-plugin.toml"),
            "name = \"plug\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        std::fs::write(
            plugin_dir.join(".claude-plugin").join("plugin.json"),
            r#"{"name": "plug", "version": "1.0.0"}"#,
        )
        .unwrap();
        for (name, body) in skills {
            let dir = plugin_dir.join("skills").join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("SKILL.md"), body).unwrap();
        }

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
        seed_catalog_enrolment(&paths, &catalog_root, "acme");
        lifecycle::enable(&id, &deps).expect("enable plugin");

        paths
    }

    /// Driving the CLI search through `query::run_with_deps` (the `pub` library
    /// entry the binary's `tome query` wraps) emits EXACTLY ONE `tome.search`
    /// with `surface == "cli"`, `calling_harness` OMITTED, a present
    /// `embedder_model_id`, and the RAW integer fields.
    #[test]
    fn cli_query_emits_search_surface_cli() {
        let home = TempDir::new().unwrap();
        let _home_guard = HomeGuard::install(home.path());
        let _env = EnvForce::install();

        let paths = stage_at_home(
            home.path(),
            &[("alpha", &skill_body("alpha", "alpha widget configuration"))],
        );
        // Point the process-global emit at this isolated queue for the test.
        let _handle = tome::telemetry::TelemetryHandleGuard::install(
            tome::telemetry::build_handle_for_test(&paths),
        );

        let scope = Scope(WorkspaceName::global());
        let config = Config::default();
        let embedder = StubEmbedder::new();
        let reranker = StubReranker::new();
        let deps = query::QueryDeps {
            paths: &paths,
            scope: &scope,
            config: &config,
            embedder: &embedder,
            reranker: Some(&reranker),
            embedder_seed: stub_embedder_seed(),
            reranker_seed: stub_reranker_seed(),
        };
        let args = QueryArgs {
            text: "alpha widget configuration".into(),
            top_k: Some(10),
            catalog: None,
            plugin: None,
            no_rerank: false,
            strict: false,
            min_score: None,
        };

        let outcome = query::run_with_deps(args, deps, Mode::Json).expect("query run_with_deps ok");
        assert!(
            !outcome.results.is_empty(),
            "the staged corpus must return at least one result so the emit reflects a real search",
        );

        let events = queue_events(&paths);
        let searches: Vec<&Value> = events
            .iter()
            .filter(|e| e["event_name"] == "tome.search")
            .collect();
        assert_eq!(
            searches.len(),
            1,
            "exactly one tome.search emitted by one CLI query: {events:?}",
        );
        let search = searches[0];
        let a = &search["attributes"];

        assert_eq!(a["surface"], "cli", "CLI surface: {search}");
        assert!(
            a.get("calling_harness").is_none(),
            "the CLI surface OMITS calling_harness (an MCP-only dimension): {search}",
        );
        assert_eq!(
            a["embedder_model_id"],
            Value::String(registry_embedder_name().to_string()),
            "embedder_model_id is the pinned registry embedder name: {search}",
        );
        // RAW integer quantities (the kernel buckets at read time).
        for field in ["latency_ms", "candidates_returned", "corpus_size"] {
            assert!(
                a.get(field).map(Value::is_number).unwrap_or(false),
                "field {field} present as a raw integer: {search}",
            );
        }
        // The bundled-local providers report `bundled` (Phase 12 closed enum).
        assert_eq!(a["embedding_provider_kind"], "bundled");
        assert_eq!(a["reranker_provider_kind"], "bundled");
        // `reranker_used` reflects the reranker we passed; `strict` reflects args.
        assert_eq!(a["reranker_used"], Value::Bool(true), "reranker passed");
        assert_eq!(a["strict"], Value::Bool(false), "non-strict query");
    }
}

// ---------------------------------------------------------------------------
// Shared in-process env force (for the `HomeGuard`-pinned process-global emit
// paths). Forces `TOME_TELEMETRY=1` + a loopback endpoint so `init` builds an
// ENABLED handle; restores on drop.
// ---------------------------------------------------------------------------

struct EnvForce {
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl EnvForce {
    fn install() -> Self {
        let saved = TELEMETRY_ENV_VARS
            .iter()
            .map(|&k| (k, std::env::var_os(k)))
            .collect::<Vec<_>>();
        // SAFETY: the caller holds `HOME_MUTEX` via a `HomeGuard` for the whole
        // test, so no other test mutates these process-global vars concurrently.
        for &k in TELEMETRY_ENV_VARS {
            unsafe { std::env::remove_var(k) };
        }
        unsafe {
            std::env::set_var("TOME_TELEMETRY", "1");
            std::env::set_var("TOME_GAUGE_ENDPOINT", LOOPBACK_ENDPOINT);
        }
        Self { saved }
    }
}

impl Drop for EnvForce {
    fn drop(&mut self) {
        for (k, v) in &self.saved {
            // SAFETY: still under the test's `HomeGuard`/`HOME_MUTEX`.
            match v {
                Some(val) => unsafe { std::env::set_var(k, val) },
                None => unsafe { std::env::remove_var(k) },
            }
        }
    }
}
