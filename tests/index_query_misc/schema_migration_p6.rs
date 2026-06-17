//! Phase 6 — the marker-only `kind`-domain migration (entry-schema-p6.md
//! § "Schema migration").
//!
//! The `skills.kind` column is free-text TEXT, so admitting the new
//! `'agent'` domain value needs no DDL and no data backfill. Phase 6 still
//! registers a `Migration` step that advances the schema version 3 → 4 so
//! doctor's schema check and the migration registry agree the domain
//! widened and the version stays monotonic and auditable.
//!
//! Unlike `tests/schema_migration_e2e.rs` (which exercises the framework
//! against synthetic migrations), this test runs the *production*
//! `MIGRATIONS` registry — no `MIGRATIONS_OVERRIDE` — so it pins the real
//! `phase_6_v3_to_v4` step.

use crate::common::write_index_db_with_schema_version;
use tempfile::TempDir;
use tome::index::SCHEMA_VERSION;
use tome::index::migrations::{apply_pending, current_schema_version};

#[test]
fn kind_domain_marker_bumps_version() {
    // Phase 6 introduced the v3→v4 marker migration. SCHEMA_VERSION has
    // since advanced (Phase 11 bumped it to 5); the assertion below pins
    // only that the compiled version is at least 4, not the exact value.
    const {
        assert!(
            SCHEMA_VERSION >= 4,
            "Phase 6 must have bumped SCHEMA_VERSION to at least 4"
        );
    }

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("index.db");
    // Stamp a synthetic v3 DB, then run the production marker migration
    // targeting v4 explicitly — this exercises the real phase_6_v3_to_v4
    // step regardless of the current compiled SCHEMA_VERSION.
    write_index_db_with_schema_version(&path, 3);

    let mut conn = rusqlite::Connection::open(&path).expect("open synthetic v3 db");
    let new_version = apply_pending(&mut conn, 3, 4).expect("marker migration runs");
    assert_eq!(new_version, 4, "apply_pending returns the new version");

    let stored = current_schema_version(&conn).expect("probe").unwrap();
    assert_eq!(stored, 4, "meta.schema_version reflects the v4 marker bump");
}
