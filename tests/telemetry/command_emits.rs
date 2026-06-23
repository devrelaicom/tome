//! Phase 10 / US2 (T-H1 / T-H2) — per-command anonymous emits, asserted at the
//! produced queue line.
//!
//! Two driving models live here, each used where it is the faithful one:
//!
//! 1. **In-process, stub-embedder** (`query::run_with_deps`): the CLI
//!    `tome.search { surface: cli }` path. The real `tome query` binary loads
//!    real ONNX models, so the only stub-only way to reach the CLI search emit
//!    is its `pub` library entry — which runs the SAME `enqueue` the binary
//!    does. The whole staged tree is rooted at `$HOME/.tome` (a `HomeGuard`
//!    pins it) and telemetry is force-enabled, so the handler's default-`Paths`
//!    enqueue lands where the test reads it. This mirrors `mcp_funnel.rs`.
//!
//! 2. **Real binary** (`ToolEnv` + `Command`): the catalog / workspace / doctor
//!    command paths, whose telemetry emits live in the CLI command WRAPPERS (not
//!    the stub-injectable library fns) and which do NOT load ONNX. Each spawned
//!    `Command` clears every CI / `TOME_TELEMETRY*` var then force-enables
//!    telemetry, exactly the `identity.rs` hygiene, so the emit isn't CI-auto-off.
//!
//! Scope notes (documented in the report):
//! - `plugin_action`: the enable/disable CLI wrappers construct a real
//!   `FastembedEmbedder` (ONNX) with no stub seam, so the enable/disable emit is
//!   NOT reachable end-to-end with stubs via the binary. The highest-value
//!   stub-only coverage is the produced-line round-trip through the gated default
//!   `enqueue` (the exact event + gate the wrapper uses), asserted below.
//! - catalog `source_type == "git"`: a git-URL `catalog add` FAILS the clone
//!   (exit 6) before reaching the success-gated emit, so the `Git` branch value
//!   is unreachable via a real clone. We cover the `Local` branch end-to-end AND
//!   the success-gate (a failed add emits NOTHING); the `Git` enum VALUE is
//!   pinned by the `events.rs` `CatalogActionEvent { SourceType::Git }` line.

use std::process::Command;

use serde_json::Value;

use crate::common::{Fixture, ToolEnv};

// ---------------------------------------------------------------------------
// Env hygiene — shared by the real-binary tests below.
// ---------------------------------------------------------------------------

/// Every env var that can flip the telemetry enabled-state precedence. Cleared
/// on every spawned `Command` for a deterministic (force-on) baseline. Mirrors
/// the `identity` / `inspect` suites' list.
const TELEMETRY_ENV_VARS: &[&str] = &[
    "TOME_TELEMETRY",
    "TOME_TELEMETRY_ENDPOINT",
    "CI",
    "GITHUB_ACTIONS",
    "GITLAB_CI",
    "CIRCLECI",
    "BUILDKITE",
    "JENKINS_URL",
    "TF_BUILD",
    "TEAMCITY_VERSION",
];

/// A `tome` command over the isolated `$HOME`, every CI/telemetry var removed,
/// then telemetry FORCE-ENABLED (`TOME_TELEMETRY=1`, overriding CI auto-off) and
/// pointed at a non-routable endpoint so an accidental inline flush would fail
/// loudly rather than hang silently.
fn force_on_cmd(env: &ToolEnv) -> Command {
    let mut cmd = env.cmd();
    for &k in TELEMETRY_ENV_VARS {
        cmd.env_remove(k);
    }
    cmd.env("TOME_TELEMETRY", "1");
    cmd.env("TOME_TELEMETRY_ENDPOINT", "http://192.0.2.0:0/telemetry");
    cmd
}

/// The `telemetry/queue.jsonl` path under the isolated home.
fn queue_path(env: &ToolEnv) -> std::path::PathBuf {
    env.tome_root().join("telemetry").join("queue.jsonl")
}

/// Read the queued telemetry lines under the isolated `$HOME/.tome` as parsed
/// JSON objects. Empty when the queue file doesn't exist yet.
fn queue_events(env: &ToolEnv) -> Vec<Value> {
    let body = match std::fs::read_to_string(queue_path(env)) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    body.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<Value>(l).expect("queued line is JSON"))
        .collect()
}

/// First event of a given `event_type`, if any.
fn first_of<'a>(events: &'a [Value], event_type: &str) -> Option<&'a Value> {
    events.iter().find(|e| e["event_type"] == event_type)
}

/// Count events of a given `event_type`.
fn count_of(events: &[Value], event_type: &str) -> usize {
    events
        .iter()
        .filter(|e| e["event_type"] == event_type)
        .count()
}

// ===========================================================================
// T-H2 — catalog_action (local) + the success-gate on a failed add.
// ===========================================================================

/// `catalog add <file:// fixture>` ⇒ a `tome.catalog_action { action: added,
/// source_type: local }`. The resolved `file://` URL drives the `Local` branch.
#[test]
fn catalog_add_local_emits_catalog_action_added_local() {
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
    let ev = first_of(&events, "tome.catalog_action")
        .unwrap_or_else(|| panic!("no tome.catalog_action in queue: {events:?}"));
    assert_eq!(ev["action"], "added", "catalog add ⇒ action=added: {ev}");
    assert_eq!(
        ev["source_type"], "local",
        "a file:// source resolves to source_type=local: {ev}"
    );
}

/// A FAILED `catalog add` (a git URL pointing at nothing → exit 6) emits NO
/// `tome.catalog_action` — the emit is gated on a successful add. This proves the
/// success-gate (a dropped or ungated emit would surface a spurious line); the
/// `Git` source_type VALUE itself is pinned in `events.rs`.
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
        count_of(&events, "tome.catalog_action"),
        0,
        "a failed catalog add must emit no catalog_action (success-gated): {events:?}",
    );
}

// ===========================================================================
// T-H2 — workspace_action: init emits; a no-op `use` does NOT (emit_on_ok gate).
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
    let ev = first_of(&events, "tome.workspace_action")
        .unwrap_or_else(|| panic!("no tome.workspace_action in queue: {events:?}"));
    assert_eq!(ev["action"], "init", "workspace init ⇒ action=init: {ev}");
}

/// A no-op `workspace use <nonexistent>` (exit 13, WorkspaceNotFound) emits NO
/// `tome.workspace_action` — proving the `emit_on_ok` success-gate (the verb only
/// emits when the mutation actually happened).
#[test]
fn noop_workspace_use_emits_no_workspace_action() {
    let env = ToolEnv::new();
    // Run from an isolated CWD so a stray bind can't pollute the test process's
    // working dir (a failed `use` writes nothing, but be defensive).
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
        count_of(&events, "tome.workspace_action"),
        0,
        "a failed `workspace use` must emit nothing (emit_on_ok gate): {events:?}",
    );
}

// ===========================================================================
// T-H2 — doctor_run: a `tome doctor` invocation emits the run event.
// ===========================================================================

/// `tome doctor` ⇒ a `tome.doctor_run { fix: false, findings_bucket: <bucket> }`.
/// `doctor` may classify a fresh home as degraded (exit 1), but the emit fires
/// BEFORE the exit path, so it lands regardless of the exit code.
#[test]
fn doctor_emits_doctor_run_with_fix_flag_and_findings_bucket() {
    let env = ToolEnv::new();

    let out = force_on_cmd(&env)
        .args(["doctor"])
        .output()
        .expect("spawn tome");
    // Exit may be 0 or 1 (degraded) depending on the fresh-home report; either
    // way the emit happened before the exit branch.
    let code = out.status.code();
    assert!(
        code == Some(0) || code == Some(1),
        "doctor should exit 0 or 1 (degraded), got {code:?}; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    let events = queue_events(&env);
    let ev = first_of(&events, "tome.doctor_run")
        .unwrap_or_else(|| panic!("no tome.doctor_run in queue: {events:?}"));
    assert_eq!(
        ev["fix"],
        Value::Bool(false),
        "doctor (no --fix) ⇒ fix=false: {ev}"
    );
    assert!(
        ev.get("findings_bucket").and_then(Value::as_str).is_some(),
        "doctor_run carries a findings_bucket string: {ev}"
    );
}

/// `tome doctor --fix` ⇒ a `tome.doctor_run { fix: true, .. }`. Pins the `fix`
/// bool flips with the flag (the other half of the bucketed-emit contract).
#[test]
fn doctor_fix_emits_doctor_run_with_fix_true() {
    let env = ToolEnv::new();

    let out = force_on_cmd(&env)
        .args(["doctor", "--fix"])
        .output()
        .expect("spawn tome");
    let code = out.status.code();
    // `--fix` may exit 0, 1, or 75 (fix ran, manual work remains) on a fresh
    // home; the emit fires before any of those exit branches.
    assert!(
        matches!(code, Some(0) | Some(1) | Some(75)),
        "doctor --fix exit {code:?}; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    let events = queue_events(&env);
    let ev = first_of(&events, "tome.doctor_run")
        .unwrap_or_else(|| panic!("no tome.doctor_run in queue: {events:?}"));
    assert_eq!(
        ev["fix"],
        Value::Bool(true),
        "doctor --fix ⇒ fix=true: {ev}"
    );
}

// ===========================================================================
// T-H2 — plugin_action: the gated default-`enqueue` round-trip (stub-only).
//
// The enable/disable CLI wrappers load real ONNX with no stub seam, so the
// enable/disable emit is NOT reachable end-to-end with stubs via the binary.
// This asserts the EXACT events those wrappers emit (`PluginActionEvent` with
// `Enabled` / `Disabled`) round-trip through the gated default-`Paths`
// `enqueue` — the produced-line + enabled-gate primitive the wrappers depend on,
// which the binary path can't exercise in fast CI.
// ===========================================================================

#[test]
fn plugin_action_enqueue_round_trips_enabled_and_disabled() {
    use crate::common::HomeGuard;
    use tome::paths::Paths;
    use tome::telemetry::event::{PluginAction, PluginActionEvent};

    let home = tempfile::TempDir::new().unwrap();
    let _home_guard = HomeGuard::install(home.path());
    let _env = EnvForce::install();

    // Both variants through the REAL default-Paths gated `enqueue` (the exact
    // call the CLI wrappers make).
    tome::telemetry::enqueue(PluginActionEvent {
        action: PluginAction::Enabled,
    });
    tome::telemetry::enqueue(PluginActionEvent {
        action: PluginAction::Disabled,
    });

    let paths = Paths::from_root(home.path().join(".tome"));
    let lines = tome::telemetry::queue::read_lines(&paths).expect("read queue");
    let events: Vec<Value> = lines
        .iter()
        .map(|l| serde_json::from_str(l).expect("json"))
        .collect();

    let actions: Vec<&str> = events
        .iter()
        .filter(|e| e["event_type"] == "tome.plugin_action")
        .map(|e| e["action"].as_str().expect("action string"))
        .collect();
    assert_eq!(
        actions,
        vec!["enabled", "disabled"],
        "both plugin_action variants land with the right action tokens: {events:?}",
    );
}

// ===========================================================================
// T-H1 — CLI `tome.search { surface: cli }` end-to-end via the library entry.
//
// Unix-only: the staging symlinks the catalog cache dir (the same shape
// `mcp_funnel.rs` uses), so this section is gated like its peer.
// ===========================================================================

#[cfg(unix)]
mod cli_search {
    use super::*;

    use std::path::Path;

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

    /// The pinned registry embedder name — the value `Search.embedder_model_id`
    /// carries (`Some(embedder_entry().name)`), derived from the public registry
    /// so the test doesn't need the `pub(crate)` accessor.
    fn registry_embedder_name() -> &'static str {
        // The telemetry `embedder_model_id` reports the DEFAULT profile's pinned
        // embedder (B4: a non-registry stub seed falls back to the default).
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

    /// Stage `acme/plug` with the supplied skills rooted at `home/.tome`, enabled
    /// + indexed against `global` with the StubEmbedder. Mirrors `mcp_funnel.rs`.
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
    /// with `surface == "cli"`, `calling_harness` OMITTED (a CLI-only-absent
    /// dimension), a present `embedder_model_id`, and the bucketed fields.
    #[test]
    fn cli_query_emits_search_surface_cli() {
        let home = TempDir::new().unwrap();
        let _home_guard = HomeGuard::install(home.path());
        let _env = EnvForce::install();

        let paths = stage_at_home(
            home.path(),
            &[("alpha", &skill_body("alpha", "alpha widget configuration"))],
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

        // The emit landed at the default-`$HOME` queue (HomeGuard-pinned to the
        // staged home), so it reads back here.
        let lines = tome::telemetry::queue::read_lines(&paths).expect("read queue");
        let searches: Vec<Value> = lines
            .iter()
            .map(|l| serde_json::from_str::<Value>(l).expect("json line"))
            .filter(|e| e["event_type"] == "tome.search")
            .collect();
        assert_eq!(
            searches.len(),
            1,
            "exactly one tome.search emitted by one CLI query: {lines:?}",
        );
        let search = &searches[0];

        assert_eq!(search["surface"], "cli", "CLI surface: {search}");
        assert!(
            search.get("calling_harness").is_none(),
            "the CLI surface OMITS calling_harness (an MCP-only dimension): {search}",
        );
        assert_eq!(
            search["embedder_model_id"],
            Value::String(registry_embedder_name().to_string()),
            "embedder_model_id is the pinned registry embedder name: {search}",
        );
        // The bucketed fields are present (closed-enum string tokens).
        for field in [
            "latency_bucket",
            "candidates_returned",
            "corpus_size_bucket",
        ] {
            assert!(
                search.get(field).and_then(Value::as_str).is_some(),
                "bucketed field {field} present as a string: {search}",
            );
        }
        // `reranker_used` reflects the reranker we passed; `strict` reflects args.
        assert_eq!(
            search["reranker_used"],
            Value::Bool(true),
            "reranker passed"
        );
        assert_eq!(search["strict"], Value::Bool(false), "non-strict query");
    }
}

// ---------------------------------------------------------------------------
// Shared in-process env force (for the `HomeGuard`-pinned default-`Paths` emit
// paths). Lifted from the `mcp_funnel.rs` / `queue_behavior.rs` shape.
// ---------------------------------------------------------------------------

/// Snapshot + clear the telemetry/CI env vars, force `TOME_TELEMETRY=1` plus a
/// non-routable endpoint, restore everything on drop. Pairs with a `HomeGuard`
/// (held for the whole test) so the env mutation can't race a sibling.
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
            std::env::set_var("TOME_TELEMETRY_ENDPOINT", "http://192.0.2.0:0/telemetry");
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
