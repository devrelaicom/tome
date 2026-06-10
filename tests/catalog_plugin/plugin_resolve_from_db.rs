//! FF1 regression: `resolve_plugin_dir` must resolve a plugin from the
//! `workspace_catalogs` DB enrolment, NOT `config.toml [catalogs]`.
//!
//! Why this file exists: `tome catalog add` enrols a catalog ONLY into the
//! `workspace_catalogs` SQLite table — it never writes `config.toml
//! [catalogs]` (`catalog::store::save` has zero callers in `src/`). Before
//! FF1, every `resolve_plugin_dir`-driven command (`plugin enable/disable/
//! show`, `reindex`, `catalog update`) read `config.catalogs` first and so
//! failed with exit 3 (`CatalogNotFound`) on a *fresh install* that has no
//! `config.toml` — the exact state a real user lands in. The whole test
//! suite hid this because the shared `config_with_catalog` helper dual-wrote
//! config AND the DB.
//!
//! These tests deliberately set up the enrolment via the REAL production
//! flow (`tome catalog add file://<git-fixture>`) — NO `config.toml` is ever
//! written — and assert that the resolution-driven commands now SUCCEED.

use crate::common::{
    Fixture, ToolEnv, fabricate_models, has_global_enrolment, paths_for,
    sample_plugin_catalog_fixture,
};
use tome::commands::plugin::registry_seeds;
use tome::embedding::stub::StubEmbedder;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

/// Build a git repo from the `sample-plugin-catalog` skeleton, enrol it via
/// the real `tome catalog add` (DB-only — no config.toml), and fabricate
/// model manifests so the later library `enable` is satisfied. Returns the
/// `Fixture` (keeps the git remote alive) and the isolated `ToolEnv`.
fn add_catalog_no_config() -> (Fixture, ToolEnv) {
    let fix = Fixture::build_from(sample_plugin_catalog_fixture());
    let env = ToolEnv::new();

    let out = env
        .cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .expect("spawn catalog add");
    assert!(
        out.status.success(),
        "catalog add failed: exit={:?} stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    let paths = paths_for(&env);
    // Production invariant under test: enrolment lands in the DB...
    assert!(
        has_global_enrolment(&paths, "sample-plugin-catalog"),
        "catalog add must enrol into workspace_catalogs",
    );
    // ...and `config.toml` is NOT written (the source of the bug).
    assert!(
        !paths.global_config_file.exists(),
        "catalog add must NOT write config.toml; resolution must work without it",
    );

    fabricate_models(&paths);
    (fix, env)
}

/// End-to-end happy path through the resolution chokepoint: enable (library,
/// stub embedder) → `plugin show` (CLI) → `plugin disable` (CLI), every step
/// resolving the plugin dir from the DB enrolment with zero `config.toml`.
#[test]
fn catalog_add_then_enable_show_disable_resolve_from_db_without_config() {
    let (_fix, env) = add_catalog_no_config();
    let paths = paths_for(&env);

    // Enable via the library API with the StubEmbedder. The identity SEEDS
    // must match what the `catalog add` binary stamped into `meta` (registry
    // seeds); the embedder COMPUTE is independent of those seeds, so a stub
    // embedder against registry seeds opens the DB cleanly.
    let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
    let embedder = StubEmbedder::new();
    let deps = LifecycleDeps {
        paths: &paths,
        scope: &crate::common::global_scope(),
        config: &tome::config::Config::default(),
        embedder: &embedder,
        embedder_seed,
        reranker_seed,
        summariser_seed,
        allow_model_download: false,
    };
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();

    // BEFORE FF1 this returned `CatalogNotFound` (exit 3) because the old
    // resolver read the empty `config.catalogs`. Now it resolves from the DB.
    let outcome = lifecycle::enable(&id, &deps)
        .expect("enable must resolve the plugin from the DB enrolment, not config.toml");
    assert!(
        outcome.summary.total_skills > 0,
        "enable should index the fixture's skills",
    );

    // `plugin show` via the CLI binary — read-only, no model load. Must exit
    // 0 (NOT 3) and report the plugin as enabled.
    let show = env
        .cmd()
        .args([
            "plugin",
            "show",
            "sample-plugin-catalog/plugin-alpha",
            "--json",
        ])
        .output()
        .expect("spawn plugin show");
    assert_eq!(
        show.status.code(),
        Some(0),
        "plugin show must resolve from the DB (exit 0), not exit 3; stderr: {}",
        String::from_utf8_lossy(&show.stderr),
    );
    let record: serde_json::Value =
        serde_json::from_slice(&show.stdout).expect("plugin show --json record");
    assert_eq!(record["status"], "enabled");

    // `plugin disable --force` via the CLI binary — also resolution-driven.
    let disable = env
        .cmd()
        .args([
            "plugin",
            "disable",
            "sample-plugin-catalog/plugin-alpha",
            "--force",
        ])
        .output()
        .expect("spawn plugin disable");
    assert_eq!(
        disable.status.code(),
        Some(0),
        "plugin disable must resolve from the DB (exit 0), not exit 3; stderr: {}",
        String::from_utf8_lossy(&disable.stderr),
    );
}

/// `plugin show` on a freshly-added catalog (never enabled) must still
/// resolve and exit 0 — proving resolution is independent of any prior
/// index state and reads purely from the DB enrolment.
#[test]
fn plugin_show_resolves_unenabled_plugin_from_db_without_config() {
    let (_fix, env) = add_catalog_no_config();

    let show = env
        .cmd()
        .args(["plugin", "show", "sample-plugin-catalog/plugin-beta"])
        .output()
        .expect("spawn plugin show");
    assert_eq!(
        show.status.code(),
        Some(0),
        "plugin show must resolve an un-enabled plugin from the DB; stderr: {}",
        String::from_utf8_lossy(&show.stderr),
    );
}

/// A catalog absent from BOTH the DB and `config.toml` must STILL exit 3 —
/// the fix narrows the lookup to the DB; it does not weaken the
/// `CatalogNotFound` contract for genuinely-unknown catalogs.
#[test]
fn unknown_catalog_still_exits_3_after_db_migration() {
    let (_fix, env) = add_catalog_no_config();

    let show = env
        .cmd()
        .args(["plugin", "show", "ghost-catalog/plugin-alpha"])
        .output()
        .expect("spawn plugin show");
    assert_eq!(
        show.status.code(),
        Some(3),
        "unknown catalog must remain CatalogNotFound (exit 3); stderr: {}",
        String::from_utf8_lossy(&show.stderr),
    );
}
