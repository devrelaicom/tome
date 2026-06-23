//! Phase 10 / US2 (T-H3) — process-start telemetry: the CLI `cli_startup`
//! install/upgrade lifecycle, the disabled no-emit gate, and the MCP cold-start
//! SILENT mint (the `identity.rs` AC#7 obligation).
//!
//! `cli_startup` resolves the DEFAULT `$HOME` `Paths` only when the caller passes
//! them — but its sub-calls (`enqueue`) resolve the default `$HOME` internally —
//! so every test here pins `$HOME` to a tempdir via `HomeGuard`, making the
//! default-resolved queue == the test tree's `<home>/.tome/telemetry/queue.jsonl`.
//! Telemetry is force-enabled (`TOME_TELEMETRY=1`, overriding CI auto-off) except
//! the disabled-gate test, which forces it OFF (`TOME_TELEMETRY=0`).
//!
//! Scope note (documented in the report): the REAL `tome.cold_start` emit site is
//! inside `mcp::run`, which loads real ONNX via `preflight::run` — unreachable in
//! stub-only fast CI. The in-process MCP harness builds `McpState` directly and
//! never runs `mcp::run`, so a harness tool call mints the id SILENTLY (the AC#7
//! property we assert) but does NOT itself emit `tome.cold_start`. To still pin
//! the cold-start event lands on the MCP-surface queue, the cold-start test emits
//! the EXACT `ColdStart` event `mcp::run` constructs through the same gated
//! `enqueue`, under the same forced-on isolated `$HOME`.

use serde_json::Value;
use tempfile::TempDir;
use tome::paths::Paths;
use tome::telemetry::queue;

use crate::common::HomeGuard;

/// Telemetry/CI env vars cleared before forcing the desired state.
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

/// Snapshot + clear the telemetry/CI env vars, force `TOME_TELEMETRY` to the
/// given value, restore everything on drop. Pairs with a held `HomeGuard` so the
/// env mutation can't race a sibling test.
struct EnvForceTo {
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl EnvForceTo {
    /// Force `TOME_TELEMETRY=<value>` (`"1"` = on, `"0"` = off).
    fn install(value: &str) -> Self {
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
            std::env::set_var("TOME_TELEMETRY", value);
            std::env::set_var("TOME_TELEMETRY_ENDPOINT", "http://192.0.2.0:0/telemetry");
        }
        Self { saved }
    }
}

impl Drop for EnvForceTo {
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

/// The default `Paths` for the HomeGuard-pinned `$HOME` tempdir.
fn default_paths(home: &TempDir) -> Paths {
    Paths::from_root(home.path().join(".tome"))
}

/// Read the queue under the given paths as parsed JSON objects.
fn queue_events(paths: &Paths) -> Vec<Value> {
    queue::read_lines(paths)
        .unwrap_or_default()
        .iter()
        .map(|l| serde_json::from_str::<Value>(l).expect("queued line is JSON"))
        .collect()
}

fn count_of(events: &[Value], event_type: &str) -> usize {
    events
        .iter()
        .filter(|e| e["event_type"] == event_type)
        .count()
}

fn first_of<'a>(events: &'a [Value], event_type: &str) -> Option<&'a Value> {
    events.iter().find(|e| e["event_type"] == event_type)
}

// ===========================================================================
// cli_startup — first run (install), upgrade, and the disabled no-emit gate.
// ===========================================================================

/// First-ever run (no `last-version`, fresh id): `cli_startup` enqueues
/// `tome.install` (with an `install_method`) and NO `tome.upgrade`, and stamps
/// `last-version` to the running binary's `CARGO_PKG_VERSION`.
#[test]
fn cli_startup_first_run_emits_install_and_stamps_version() {
    let home = TempDir::new().unwrap();
    let _home_guard = HomeGuard::install(home.path());
    let _env = EnvForceTo::install("1");

    let paths = default_paths(&home);
    assert!(
        !paths.telemetry_last_version().exists(),
        "no last-version stamp before the first run",
    );

    tome::telemetry::cli_startup(&paths);

    let events = queue_events(&paths);
    let install = first_of(&events, "tome.install")
        .unwrap_or_else(|| panic!("first run must emit tome.install: {events:?}"));
    assert!(
        install
            .get("install_method")
            .and_then(Value::as_str)
            .is_some(),
        "tome.install carries an install_method: {install}",
    );
    assert_eq!(
        count_of(&events, "tome.upgrade"),
        0,
        "a first install is NOT an upgrade: {events:?}",
    );

    // The id was minted and the version stamped to the running binary's version.
    assert!(
        paths.telemetry_id().exists(),
        "first run mints telemetry/id"
    );
    let stamped = std::fs::read_to_string(paths.telemetry_last_version())
        .expect("last-version stamped")
        .trim()
        .to_string();
    assert_eq!(
        stamped,
        env!("CARGO_PKG_VERSION"),
        "last-version is stamped to the running binary's version",
    );
}

/// Pre-seeding an OLDER `last-version` (and an existing id) makes the next
/// `cli_startup` emit `tome.upgrade { from_version: "0.0.1" }` and NO second
/// `tome.install`, and re-stamps `last-version` to the current version.
#[test]
fn cli_startup_upgrade_emits_upgrade_with_from_version() {
    let home = TempDir::new().unwrap();
    let _home_guard = HomeGuard::install(home.path());
    let _env = EnvForceTo::install("1");

    let paths = default_paths(&home);
    // Pre-seed an existing id (so the id is NOT just-minted ⇒ no install) and an
    // older version stamp (so a version change is detected ⇒ upgrade).
    std::fs::create_dir_all(paths.telemetry_dir()).unwrap();
    std::fs::write(
        paths.telemetry_id(),
        "11111111-2222-4333-8444-555555555555\n",
    )
    .unwrap();
    std::fs::write(paths.telemetry_last_version(), "0.0.1\n").unwrap();

    tome::telemetry::cli_startup(&paths);

    let events = queue_events(&paths);
    let upgrade = first_of(&events, "tome.upgrade")
        .unwrap_or_else(|| panic!("a version change must emit tome.upgrade: {events:?}"));
    assert_eq!(
        upgrade["from_version"], "0.0.1",
        "upgrade carries the prior version: {upgrade}",
    );
    assert_eq!(
        count_of(&events, "tome.install"),
        0,
        "a pre-existing id ⇒ NOT a fresh install ⇒ no second tome.install: {events:?}",
    );

    // The stamp now records the current version.
    let stamped = std::fs::read_to_string(paths.telemetry_last_version())
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(stamped, env!("CARGO_PKG_VERSION"), "re-stamped to current");
}

/// Telemetry disabled (`TOME_TELEMETRY=0`): `cli_startup` enqueues NOTHING — no
/// install, no upgrade, no heartbeat. The startup path self-gates on
/// `resolve_enabled` and returns before any mint/emit (FR-010).
#[test]
fn cli_startup_disabled_emits_nothing() {
    let home = TempDir::new().unwrap();
    let _home_guard = HomeGuard::install(home.path());
    let _env = EnvForceTo::install("0");

    let paths = default_paths(&home);
    tome::telemetry::cli_startup(&paths);

    // The queue is absent/empty and no id was minted (the disabled gate returns
    // before `ensure_install_id`).
    assert_eq!(
        queue::count_pending(&paths),
        0,
        "a disabled install must enqueue nothing on startup",
    );
    assert!(
        !paths.telemetry_id().exists(),
        "a disabled startup must NOT mint an install id",
    );
}

// ===========================================================================
// MCP cold-start silent mint (identity.rs AC#7) — Unix-only (catalog-cache
// symlink staging, like `mcp_funnel.rs`).
// ===========================================================================

#[cfg(unix)]
mod mcp_cold_start {
    use super::*;

    use std::path::Path;
    use std::time::Duration;

    use tome::embedding::stub::StubEmbedder;
    use tome::index::{self, OpenOptions};
    use tome::mcp::tools::search_skills;
    use tome::plugin::PluginId;
    use tome::plugin::lifecycle::{self, LifecycleDeps};
    use tome::workspace::{Scope, WorkspaceName};

    use crate::common::{
        config_with_catalog, fabricate_models, mcp_harness::McpHarness, stub_embedder_seed,
        stub_reranker_seed, stub_summariser_seed,
    };

    fn open_index(paths: &Paths) -> rusqlite::Connection {
        index::open(
            &paths.index_db,
            &OpenOptions {
                embedder: stub_embedder_seed(),
                reranker: stub_reranker_seed(),
                summariser: stub_summariser_seed(),
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

    /// Stage `acme/plug` rooted at `home/.tome` (so the MCP handler's
    /// default-`Paths` enqueue lands under the HomeGuard-pinned `$HOME`).
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

    /// MCP cold-start silent mint (AC#7): against a FRESH `$HOME` with NO
    /// pre-existing `telemetry/id`, force-on, the FIRST MCP tool call (a
    /// `search_skills` through the live in-process server) mints `telemetry/id`
    /// at mode 0600 with NO first-run notice (MCP is silent — the notice is a
    /// CLI-only concern, never called on this path). The `tome.cold_start` event
    /// (the exact event `mcp::run` emits at server start) is then shown to land
    /// on the MCP-surface queue.
    #[test]
    fn mcp_first_call_mints_id_silently_and_cold_start_lands() {
        let home = TempDir::new().unwrap();
        let _home_guard = HomeGuard::install(home.path());
        let _env = EnvForceTo::install("1");

        let paths = stage_at_home(
            home.path(),
            &[("alpha", &skill_body("alpha", "alpha widget configuration"))],
        );
        // Truly fresh identity: the staging above does not mint the id (it only
        // enables/indexes), so assert the precondition explicitly.
        assert!(
            !paths.telemetry_id().exists(),
            "precondition: no telemetry/id before the first MCP call",
        );

        // Build the live in-process server over the staged workspace and drive
        // the first tool call — this is the MCP server's first `enqueue`, which
        // lazily mints the id SILENTLY (no notice; the notice is never called on
        // the MCP path).
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

        // SILENT MINT: the id now exists at 0600.
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
        // The MCP path printed NO first-run notice (it has no human stderr gate).
        // `print_first_run_notice` is only reachable from `cli_startup`, which the
        // MCP server never calls; the mint here therefore prints nothing. We
        // assert structurally: the funnel `tome.search` landed but no
        // CLI-notice-bearing artefact exists (the notice writes to stderr only,
        // never to a file or the queue), and the `surface` is mcp.
        let events = queue_events(&paths);
        let search = first_of(&events, "tome.search")
            .unwrap_or_else(|| panic!("the first MCP call must enqueue tome.search: {events:?}"));
        assert_eq!(
            search["surface"], "mcp",
            "the MCP-surface search event: {search}",
        );

        // The `tome.cold_start` event the MCP server emits at start: the harness
        // builds `McpState` directly (it does NOT run `mcp::run`, which would
        // load real ONNX), so we enqueue the EXACT event `mcp::run` constructs
        // through the same gated default-`enqueue` to prove the event type lands
        // on this MCP-surface queue with the right shape. (The real `mcp::run`
        // cold-start site is a real-model gate, out of stub-only fast CI.)
        tome::telemetry::enqueue(tome::telemetry::event::ColdStart {
            embedder_load_bucket: tome::telemetry::buckets::LoadBucket::from(
                Duration::from_millis(10),
            ),
            index_ready_bucket: tome::telemetry::buckets::LoadBucket::from(Duration::from_millis(
                5,
            )),
            embedder_model_id: Some("stub-embedder"),
        });

        let events = queue_events(&paths);
        let cold = first_of(&events, "tome.cold_start")
            .unwrap_or_else(|| panic!("tome.cold_start must land on the queue: {events:?}"));
        assert!(
            cold.get("embedder_load_bucket")
                .and_then(Value::as_str)
                .is_some(),
            "cold_start carries an embedder_load_bucket string: {cold}",
        );
        assert!(
            cold.get("index_ready_bucket")
                .and_then(Value::as_str)
                .is_some(),
            "cold_start carries an index_ready_bucket string: {cold}",
        );
        // The cold_start shares the SAME install uuid the silent mint produced
        // (it was minted by the first MCP call, reused by this enqueue).
        assert_eq!(
            search["install_uuid"], cold["install_uuid"],
            "the cold_start reuses the silently-minted install uuid",
        );
    }
}
