//! Phase (metadata) — schema v6 → v7 `plugin_keywords` migration.
//!
//! Unlike `tests/schema_migration_e2e.rs` (which exercises the framework
//! against synthetic migrations), this test runs the *production*
//! `MIGRATIONS` registry — no `MIGRATIONS_OVERRIDE` — so it pins the real
//! `phase_meta_v6_to_v7` step. Mirrors the `schema_migration_p6.rs`
//! precedent.

use crate::common::write_index_db_with_schema_version;
use tempfile::TempDir;
use tome::index::SCHEMA_VERSION;
use tome::index::migrations::{apply_pending, current_schema_version};

#[test]
fn v6_to_v7_creates_plugin_keywords() {
    const {
        assert!(
            SCHEMA_VERSION >= 7,
            "Task D1 must have bumped SCHEMA_VERSION to at least 7"
        );
    }

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("index.db");
    // Stamp a synthetic v6 DB (meta-only, per write_index_db_with_schema_version's
    // contract), then run the production migration targeting v7 explicitly —
    // this exercises the real phase_meta_v6_to_v7 step regardless of the
    // current compiled SCHEMA_VERSION.
    write_index_db_with_schema_version(&path, 6);

    let mut conn = rusqlite::Connection::open(&path).expect("open synthetic v6 db");
    let new_version = apply_pending(&mut conn, 6, 7).expect("plugin_keywords migration runs");
    assert_eq!(new_version, 7, "apply_pending returns the new version");

    let stored = current_schema_version(&conn).expect("probe").unwrap();
    assert_eq!(stored, 7, "meta.schema_version reflects the v7 bump");

    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'plugin_keywords'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    assert!(exists, "v6 → v7 migration must create plugin_keywords");
}
