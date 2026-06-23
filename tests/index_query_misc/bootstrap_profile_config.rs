//! Task 12: `[models] profile` in `~/.tome/config.toml` seeds the
//! `model_profile` meta row on a fresh index bootstrap.
//!
//! Drives `workspace::init::init` directly via the library API so the
//! bootstrap path is exercised end-to-end without spawning a subprocess.

use crate::common::{ToolEnv, paths_for};
use tome::workspace::WorkspaceName;

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
