//! Task 12: `[models] profile` in `~/.tome/config.toml` seeds the
//! `model_profile` meta row on a fresh index bootstrap.
//!
//! The config profile is resolved at the ONE chokepoint in `db::open` so ALL
//! first-write commands (`catalog add`, `plugin enable`, `reindex`, …) honour
//! it without per-caller changes.  The tests below drive the path both via
//! `workspace::init` and via the bare `index::open` helper (simulating any
//! non-init first command), and verify the existing-DB invariant (config
//! edits after bootstrap must not re-stamp the stored profile).

use crate::common::{ToolEnv, paths_for};
use tome::index::{MetaSeed, OpenOptions};
use tome::workspace::WorkspaceName;

fn stub_seed(name: &str) -> MetaSeed {
    MetaSeed {
        name: name.to_string(),
        version: "0.0.0".to_string(),
    }
}

fn stub_open_opts() -> OpenOptions {
    OpenOptions {
        embedder: stub_seed("stub-embedder"),
        reranker: stub_seed("stub-reranker"),
        summariser: stub_seed("stub-summariser"),
        profile: None, // the field under test — must be resolved from config
    }
}

// ── via workspace::init (original coverage) ──────────────────────────────────

/// When `[models] profile = "small"` is in config, a fresh index seeds
/// `model_profile = "small"`, not the hardcoded medium default.
#[test]
fn fresh_index_uses_config_profile_small() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    // Write [models] profile = "small" to config.
    std::fs::write(&paths.global_config_file, "[models]\nprofile = \"small\"\n").unwrap();

    // Bootstrap the index via workspace init (the first call creates the DB).
    let name = WorkspaceName::parse("myws").unwrap();
    tome::workspace::init::init(name, false, &paths).expect("init");

    // Read back the stored model_profile from the meta table.
    let conn = tome::index::open_read_only(&paths.index_db).expect("read-only open");
    let stored: String = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'model_profile'",
            [],
            |r| r.get(0),
        )
        .expect("model_profile row");

    assert_eq!(
        stored, "small",
        "fresh index must seed model_profile from config; got {stored:?}"
    );
}

/// When no `[models] profile` in config, a fresh index seeds the default (medium).
#[test]
fn fresh_index_defaults_to_medium_without_config() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    // No config written.

    let name = WorkspaceName::parse("ws2").unwrap();
    tome::workspace::init::init(name, false, &paths).expect("init");

    let conn = tome::index::open_read_only(&paths.index_db).expect("read-only open");
    let stored: String = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'model_profile'",
            [],
            |r| r.get(0),
        )
        .expect("model_profile row");

    assert_eq!(
        stored, "medium",
        "fresh index without config must default to medium; got {stored:?}"
    );
}

// ── via bare index::open (non-workspace-init first command) ──────────────────

/// Critical T12 regression: the config profile is resolved at the `db::open`
/// chokepoint, so ANY command that first touches the DB (not just `workspace
/// init`) reads `[models] profile` from config.
///
/// This test bootstraps via `index::open` directly with `profile: None`
/// (mirroring `catalog add`, `plugin enable`, `reindex`, and every other
/// write command) and asserts that `active_profile == Small` when the config
/// says `small`.
#[test]
fn non_init_command_path_uses_config_profile_small() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    // Write [models] profile = "small" to config before the DB exists.
    std::fs::write(&paths.global_config_file, "[models]\nprofile = \"small\"\n").unwrap();

    // Bootstrap via the bare open helper with profile: None — this is what
    // every non-workspace-init command does (catalog add, plugin enable, …).
    let _conn = tome::index::open(&paths.index_db, &stub_open_opts()).expect("open fresh db");

    // Read active profile via the meta accessor.
    let conn = tome::index::open_read_only(&paths.index_db).expect("read-only open");
    let profile = tome::index::meta::active_profile(&conn).expect("active_profile");
    assert_eq!(
        profile,
        tome::embedding::profile::Profile::Small,
        "config [models] profile = 'small' must govern bootstrap even when the \
         first command is not `workspace init`"
    );
}

/// Invariant: an already-bootstrapped DB keeps its stored `active_profile`;
/// a later `config.toml` edit must NOT re-embed or overwrite it.
///
/// Bootstrap with `profile = "small"`, then change config to `large`, re-open
/// the DB, and assert the profile is still `Small`.
#[test]
fn existing_db_profile_not_overwritten_by_config_change() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    // Step 1: bootstrap with profile = "small".
    std::fs::write(&paths.global_config_file, "[models]\nprofile = \"small\"\n").unwrap();
    let _conn = tome::index::open(&paths.index_db, &stub_open_opts()).expect("bootstrap small");
    drop(_conn);

    // Step 2: change config to "large".
    std::fs::write(&paths.global_config_file, "[models]\nprofile = \"large\"\n").unwrap();

    // Step 3: re-open the DB — must take the migration/re-open branch, not bootstrap.
    let _conn2 = tome::index::open(&paths.index_db, &stub_open_opts()).expect("reopen");
    drop(_conn2);

    // Step 4: active_profile must still be Small (bootstrap ran once; config
    // change did not re-stamp it).
    let conn = tome::index::open_read_only(&paths.index_db).expect("read-only open");
    let profile = tome::index::meta::active_profile(&conn).expect("active_profile");
    assert_eq!(
        profile,
        tome::embedding::profile::Profile::Small,
        "existing DB profile must not be overwritten when config.toml is later edited"
    );
}
