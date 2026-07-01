//! Process-start telemetry: the CLI `cli_startup` install/upgrade lifecycle, the
//! disabled no-emit gate, and the MCP cold-start SILENT mint.
//!
//! `cli_startup` runs through the process-global handle (`HANDLE.get()`), which is
//! a set-once `OnceLock`. So the install/upgrade/disabled cases are driven through
//! the REAL `tome` binary (each subprocess gets a fresh global), reading the
//! kernel queue file the child produced. The MCP cold-start mint is driven
//! in-process via the MCP harness, whose emits route through the override-aware
//! `telemetry::emit`; a `TelemetryHandleGuard` points that at the staged queue.

use std::process::Command;

use serde_json::Value;

use crate::common::ToolEnv;
use crate::queue_util::{
    LOOPBACK_ENDPOINT, TELEMETRY_ENV_VARS, count_named, first_named, queue_events_in_root,
};

/// A `tome` command over the isolated `$HOME`, every CI/telemetry var removed,
/// then telemetry FORCE-ENABLED with a loopback endpoint.
fn force_on_cmd(env: &ToolEnv) -> Command {
    let mut cmd = env.cmd();
    for &k in TELEMETRY_ENV_VARS {
        cmd.env_remove(k);
    }
    cmd.env("TOME_TELEMETRY", "1");
    cmd.env("TOME_GAUGE_ENDPOINT", LOOPBACK_ENDPOINT);
    cmd
}

/// A `tome` command with telemetry FORCE-OFF (`TOME_TELEMETRY=0`).
fn force_off_cmd(env: &ToolEnv) -> Command {
    let mut cmd = env.cmd();
    for &k in TELEMETRY_ENV_VARS {
        cmd.env_remove(k);
    }
    cmd.env("TOME_TELEMETRY", "0");
    cmd
}

fn telemetry_dir(env: &ToolEnv) -> std::path::PathBuf {
    env.tome_root().join("telemetry")
}
fn id_path(env: &ToolEnv) -> std::path::PathBuf {
    telemetry_dir(env).join("id")
}
fn last_version_path(env: &ToolEnv) -> std::path::PathBuf {
    telemetry_dir(env).join("last-version")
}
fn queue_path(env: &ToolEnv) -> std::path::PathBuf {
    telemetry_dir(env).join("queue.jsonl")
}

fn queue_events(env: &ToolEnv) -> Vec<Value> {
    queue_events_in_root(&env.tome_root())
}

// ===========================================================================
// cli_startup — first run (install), upgrade, and the disabled no-emit gate.
// ===========================================================================

/// First-ever run (no `last-version`, fresh id): the binary's `cli_startup`
/// emits `tome.install` (with an `install_method`) and NO `tome.upgrade`, and
/// stamps `last-version` to the running binary's `CARGO_PKG_VERSION`.
#[test]
fn cli_startup_first_run_emits_install_and_stamps_version() {
    let env = ToolEnv::new();
    assert!(
        !last_version_path(&env).exists(),
        "no last-version stamp before"
    );

    let out = force_on_cmd(&env)
        .args(["catalog", "list"])
        .output()
        .expect("spawn tome");
    assert!(
        out.status.success(),
        "catalog list exited {:?}",
        out.status.code()
    );

    let events = queue_events(&env);
    let install = first_named(&events, "tome.install")
        .unwrap_or_else(|| panic!("first run must emit tome.install: {events:?}"));
    assert!(
        install["attributes"]
            .get("install_method")
            .and_then(Value::as_str)
            .is_some(),
        "tome.install carries an install_method: {install}",
    );
    assert_eq!(
        count_named(&events, "tome.upgrade"),
        0,
        "a first install is NOT an upgrade: {events:?}",
    );

    assert!(id_path(&env).exists(), "first run mints telemetry/id");
    let stamped = std::fs::read_to_string(last_version_path(&env))
        .expect("last-version stamped")
        .trim()
        .to_string();
    assert_eq!(
        stamped,
        env!("CARGO_PKG_VERSION"),
        "last-version is stamped to the running binary's version",
    );
}

/// Issue #313: on the FIRST run (fresh state, no id) the human stderr LEADS with
/// the welcome + quickstart pointer, then the required telemetry opt-out notice —
/// in that order. Both appear exactly once on this run.
#[test]
fn cli_startup_first_run_leads_with_welcome_then_notice() {
    let env = ToolEnv::new();
    assert!(
        !id_path(&env).exists(),
        "precondition: no id before first run"
    );

    let out = force_on_cmd(&env)
        .args(["catalog", "list"])
        .output()
        .expect("spawn tome");
    assert!(
        out.status.success(),
        "catalog list exited {:?}",
        out.status.code()
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    let welcome_at = stderr
        .find("Welcome to Tome!")
        .unwrap_or_else(|| panic!("first run must print the welcome: {stderr:?}"));
    let notice_at = stderr
        .find("Tome collects anonymous usage telemetry")
        .unwrap_or_else(|| panic!("first run must print the telemetry notice: {stderr:?}"));
    assert!(
        welcome_at < notice_at,
        "the welcome must LEAD the telemetry notice (welcome@{welcome_at}, notice@{notice_at}): {stderr:?}",
    );
    // The welcome points at a real starting point (the canonical first step).
    assert!(
        stderr.contains("tome catalog add"),
        "the welcome names a real entry point: {stderr:?}",
    );
    // Exactly once each (no duplicate greeting/disclosure on one run).
    assert_eq!(
        stderr.matches("Welcome to Tome!").count(),
        1,
        "welcome once: {stderr:?}"
    );
    assert_eq!(
        stderr
            .matches("Tome collects anonymous usage telemetry")
            .count(),
        1,
        "notice once: {stderr:?}",
    );
}

/// Issue #313: on a SUBSEQUENT run (id already minted) the same once-only gate
/// suppresses BOTH the welcome AND the telemetry notice — neither is re-emitted.
#[test]
fn cli_startup_subsequent_run_reprints_neither_welcome_nor_notice() {
    let env = ToolEnv::new();
    // First run mints the id and prints both lines.
    let first = force_on_cmd(&env)
        .args(["catalog", "list"])
        .output()
        .expect("spawn tome (first)");
    assert!(first.status.success());
    assert!(id_path(&env).exists(), "first run mints the id");

    // Second run over the SAME `$HOME`: the id exists, so the mint gate is closed.
    let second = force_on_cmd(&env)
        .args(["catalog", "list"])
        .output()
        .expect("spawn tome (second)");
    assert!(
        second.status.success(),
        "second catalog list exited {:?}",
        second.status.code()
    );

    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        !stderr.contains("Welcome to Tome!"),
        "the welcome must not re-print on a subsequent run: {stderr:?}",
    );
    assert!(
        !stderr.contains("Tome collects anonymous usage telemetry"),
        "the notice must not re-print on a subsequent run: {stderr:?}",
    );
}

/// Issue #313: under `--json` the human-only welcome is suppressed, but the
/// required opt-out notice still fires on first run (it goes to stderr, never
/// `--json` stdout). Structured-stdout consumers see no conversational greeting.
#[test]
fn cli_startup_first_run_json_suppresses_welcome_keeps_notice() {
    let env = ToolEnv::new();
    assert!(
        !id_path(&env).exists(),
        "precondition: no id before first run"
    );

    let out = force_on_cmd(&env)
        .args(["--json", "catalog", "list"])
        .output()
        .expect("spawn tome --json");
    assert!(
        out.status.success(),
        "catalog list --json exited {:?}",
        out.status.code()
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("Welcome to Tome!"),
        "the human welcome must be suppressed under --json: {stderr:?}",
    );
    assert!(
        stderr.contains("Tome collects anonymous usage telemetry"),
        "the required opt-out notice still fires on first run under --json: {stderr:?}",
    );
    // The welcome never lands on stdout either (that channel is JSON-only).
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("Welcome to Tome!"),
        "the welcome must never pollute --json stdout: {stdout:?}",
    );
}

/// Pre-seeding an OLDER `last-version` (and an existing id) makes the binary's
/// next `cli_startup` emit `tome.upgrade { from_version: "0.0.1" }` and NO second
/// `tome.install`, and re-stamps `last-version` to the current version.
#[test]
fn cli_startup_upgrade_emits_upgrade_with_from_version() {
    let env = ToolEnv::new();
    // Pre-seed an existing id (so the id is NOT just-minted ⇒ no install) and an
    // older version stamp (so a version change is detected ⇒ upgrade).
    std::fs::create_dir_all(telemetry_dir(&env)).unwrap();
    std::fs::write(id_path(&env), "11111111-2222-4333-8444-555555555555\n").unwrap();
    std::fs::write(last_version_path(&env), "0.0.1\n").unwrap();

    let out = force_on_cmd(&env)
        .args(["catalog", "list"])
        .output()
        .expect("spawn tome");
    assert!(
        out.status.success(),
        "catalog list exited {:?}",
        out.status.code()
    );

    let events = queue_events(&env);
    let upgrade = first_named(&events, "tome.upgrade")
        .unwrap_or_else(|| panic!("a version change must emit tome.upgrade: {events:?}"));
    assert_eq!(
        upgrade["attributes"]["from_version"], "0.0.1",
        "upgrade carries the prior version: {upgrade}",
    );
    assert_eq!(
        count_named(&events, "tome.install"),
        0,
        "a pre-existing id ⇒ NOT a fresh install ⇒ no second tome.install: {events:?}",
    );

    let stamped = std::fs::read_to_string(last_version_path(&env))
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(stamped, env!("CARGO_PKG_VERSION"), "re-stamped to current");
}

/// Telemetry disabled (`TOME_TELEMETRY=0`): the binary's startup path enqueues
/// NOTHING — no install, no upgrade, no heartbeat — and mints NO id.
#[test]
fn cli_startup_disabled_emits_nothing() {
    let env = ToolEnv::new();

    let out = force_off_cmd(&env)
        .args(["catalog", "list"])
        .output()
        .expect("spawn tome");
    assert!(
        out.status.success(),
        "catalog list exited {:?}",
        out.status.code()
    );

    assert!(
        !queue_path(&env).exists(),
        "a disabled install must enqueue nothing on startup",
    );
    assert!(
        !id_path(&env).exists(),
        "a disabled startup must NOT mint an install id",
    );
}

// ===========================================================================
// MCP cold-start silent mint — Unix-only (catalog-cache symlink staging, like
// `mcp_funnel.rs`).
// ===========================================================================

#[cfg(unix)]
mod mcp_cold_start {
    use super::*;

    use std::path::Path;

    use tempfile::TempDir;
    use tome::embedding::stub::StubEmbedder;
    use tome::index::{self, OpenOptions};
    use tome::mcp::tools::search_skills;
    use tome::paths::Paths;
    use tome::plugin::PluginId;
    use tome::plugin::lifecycle::{self, LifecycleDeps};
    use tome::workspace::{Scope, WorkspaceName};

    use crate::common::{
        HomeGuard, config_with_catalog, fabricate_models, mcp_harness::McpHarness,
        stub_embedder_seed, stub_reranker_seed, stub_summariser_seed,
    };
    use crate::queue_util::queue_events;

    /// Snapshot + clear the telemetry/CI env vars, force `TOME_TELEMETRY=1` plus a
    /// loopback endpoint, restore on drop. Pairs with a held `HomeGuard`.
    struct EnvForce {
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }

    impl EnvForce {
        fn install() -> Self {
            let saved = TELEMETRY_ENV_VARS
                .iter()
                .map(|&k| (k, std::env::var_os(k)))
                .collect::<Vec<_>>();
            // SAFETY: the caller holds `HOME_MUTEX` via a `HomeGuard`.
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

    /// MCP cold-start silent mint: against a FRESH `$HOME` with NO pre-existing
    /// `telemetry/id`, force-on, the FIRST MCP tool call (`search_skills` through
    /// the live in-process server) mints `telemetry/id` at mode 0600 with NO
    /// first-run notice (the notice is a CLI-only concern, never called on the MCP
    /// path), and lands a `surface=mcp` `tome.search`.
    #[test]
    fn mcp_first_call_mints_id_silently_and_search_lands() {
        let home = TempDir::new().unwrap();
        let _home_guard = HomeGuard::install(home.path());
        let _env = EnvForce::install();

        let paths = stage_at_home(
            home.path(),
            &[("alpha", &skill_body("alpha", "alpha widget configuration"))],
        );
        assert!(
            !paths.telemetry_id().exists(),
            "precondition: no telemetry/id before the first MCP call",
        );

        // Point the override-aware process-global emit at this staged queue so the
        // MCP handler's `telemetry::emit` lands here AND lazily mints the id.
        let _handle = tome::telemetry::TelemetryHandleGuard::install(
            tome::telemetry::build_handle_for_test(&paths),
        );

        let harness = McpHarness::new(&paths);
        let out = harness
            .call_search_skills(search_skills::Input {
                query: "alpha widget configuration".into(),
                top_k: Some(10),
                catalog: None,
                plugin: None,
                description_max_chars: Some(150),
            })
            .expect("search_skills ok");
        assert!(
            !out.matches.is_empty(),
            "the staged corpus must return a result (the funnel emit reflects a real call)",
        );

        // SILENT MINT: the id now exists at 0600 (the first emit minted it).
        assert!(
            paths.telemetry_id().exists(),
            "the first MCP tool call must lazily mint telemetry/id",
        );
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(paths.telemetry_id())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "the minted id must be 0600");
        }

        // The MCP path printed NO first-run notice (it has no human stderr gate);
        // the funnel `tome.search` landed with `surface=mcp`.
        let events = queue_events(&paths);
        let search = first_named(&events, "tome.search")
            .unwrap_or_else(|| panic!("the first MCP call must enqueue tome.search: {events:?}"));
        assert_eq!(
            search["attributes"]["surface"], "mcp",
            "the MCP-surface search event: {search}",
        );
    }
}
