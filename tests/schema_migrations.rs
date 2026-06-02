//! Phase 10 / T194 — forward-only schema migration coverage.
//!
//! `tests/index_schema_bootstrap.rs` covers the bootstrap and the
//! "stored newer than compiled → SchemaTooNew" library-level path. This
//! file adds the remaining migration-mechanism tests:
//!
//! - Reading the schema version on a fresh / bootstrapped / stamped DB.
//! - The "stored older than compiled BUT no migration row exists" case —
//!   this is the load-bearing forward-only invariant: a downgraded binary
//!   that cannot reach the compiled version refuses with a controlled
//!   `IndexIntegrityCheckFailure` rather than corrupting the DB.
//! - End-to-end exit 52 via the CLI binary when the on-disk version is
//!   higher than the compiled binary (the bootstrap test only covers the
//!   library API).
//!
//! Phase 2 ships at schema version 1 and `migrations::MIGRATIONS` is empty
//! (the v0 → v1 path is the bootstrap, not a migration row). A
//! synthetically-stamped v0 DB therefore exercises the "no registered
//! migration" guard, which is the forward-only contract's safety net.

mod common;

use common::{ToolEnv, paths_for};
use tempfile::TempDir;
use tome::error::TomeError;
use tome::index::{MetaSeed, OpenOptions, SCHEMA_VERSION, current_schema_version, open};

fn options() -> OpenOptions {
    OpenOptions {
        embedder: MetaSeed {
            name: "test-embedder".into(),
            version: "1.0".into(),
        },
        reranker: MetaSeed {
            name: "test-reranker".into(),
            version: "1.0".into(),
        },
        summariser: MetaSeed {
            name: "test-summariser".into(),
            version: "1.0".into(),
        },
    }
}

#[test]
fn current_schema_version_is_none_before_bootstrap() {
    // A freshly-created on-disk SQLite file with no meta table at all.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("index.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    let v = current_schema_version(&conn).expect("probe succeeds");
    assert_eq!(v, None, "expected None on fresh DB, got {:?}", v);
}

#[test]
fn meta_table_present_but_schema_version_row_missing_is_corruption_not_fresh() {
    // FR-015 (F-BOOT-META-DIAG): a `meta` table that EXISTS but is missing
    // its `schema_version` row is a CORRUPTION case, distinct from a fresh DB
    // (no `meta` table at all). The blanket `.ok()` previously collapsed both
    // to `Ok(None)` → "fresh DB" → a misleading "table meta already exists" on
    // re-bootstrap. The distinction must be explicit and reuse the existing
    // `IndexIntegrityCheckFailure` variant (exit 51) — no new code.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("index.db");

    // Bootstrap a real DB, then delete only the `schema_version` row so the
    // `meta` table is present but the version row is gone.
    {
        let conn = open(&path, &options()).expect("bootstrap");
        conn.execute("DELETE FROM meta WHERE key = 'schema_version'", [])
            .expect("delete schema_version row");
    }

    let conn = rusqlite::Connection::open(&path).unwrap();
    let err = current_schema_version(&conn)
        .expect_err("meta-present-but-version-missing must be corruption, not Ok(None)");
    assert_eq!(
        err.exit_code(),
        51,
        "corruption must surface IndexIntegrityCheckFailure (exit 51), got {} from {err:?}",
        err.exit_code(),
    );
    match err {
        TomeError::IndexIntegrityCheckFailure(msg) => {
            assert!(
                msg.contains("schema_version"),
                "diagnostic must name the missing row, got: {msg}",
            );
        }
        other => panic!("expected IndexIntegrityCheckFailure, got {other:?}"),
    }
}

#[test]
fn current_schema_version_matches_compiled_after_bootstrap() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("index.db");
    let conn = open(&path, &options()).expect("bootstrap");
    let v = current_schema_version(&conn).expect("probe succeeds");
    assert_eq!(v, Some(SCHEMA_VERSION));
}

#[test]
fn stamped_below_compiled_with_no_migration_registered_errors() {
    // Bootstrap normally, then manually stamp the schema_version row at
    // 0 — older than the compiled SCHEMA_VERSION (= 1). The migration
    // table is empty (Phase 2 ships with no MIGRATIONS rows, by design:
    // v0 → v1 is the bootstrap path, not a registered migration). Re-open
    // must refuse with a controlled `IndexIntegrityCheckFailure` whose
    // message names the missing step.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("index.db");
    {
        let conn = open(&path, &options()).expect("bootstrap");
        conn.execute(
            "UPDATE meta SET value = '0' WHERE key = 'schema_version'",
            [],
        )
        .expect("downgrade stamp");
    }
    let err = open(&path, &options()).expect_err("reopen must refuse");
    match err {
        TomeError::IndexIntegrityCheckFailure(msg) => {
            assert!(
                msg.contains("no migration registered for schema 0"),
                "expected 'no migration registered for schema 0 → 1', got: {msg}",
            );
        }
        other => {
            panic!("expected IndexIntegrityCheckFailure for missing migration row, got {other:?}",)
        }
    }
}

#[test]
fn reopen_at_current_version_runs_no_migrations() {
    // Sanity: a DB at SCHEMA_VERSION on disk reopens as a no-op. Already
    // covered by `index_schema_bootstrap.rs::reopen_is_noop`; re-asserted
    // here so the forward-only test surface is self-contained.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("index.db");
    {
        let _ = open(&path, &options()).expect("bootstrap");
    }
    let conn = open(&path, &options()).expect("reopen");
    let v = current_schema_version(&conn).unwrap();
    assert_eq!(v, Some(SCHEMA_VERSION));
}

#[test]
fn cli_status_exits_1_on_schema_too_new() {
    // End-to-end exit 52 is the library path's contract; surfacing it
    // through the CLI is gated by `tome status`, which opens the index
    // read-write and bubbles `SchemaTooNew` to the user. `status::run`
    // exits with code 1 because the failure makes the report meaningless
    // — but the `TomeError::SchemaTooNew` itself maps to exit 52 for the
    // commands that propagate it (those tests live alongside their
    // command files; here we just confirm `status` reports something
    // non-zero rather than silently rendering an "OK" page on a DB it
    // cannot understand).
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    {
        // Bootstrap then stamp at SCHEMA_VERSION + 1.
        let conn = open(&paths.index_db, &options()).expect("bootstrap");
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            rusqlite::params![(SCHEMA_VERSION + 1).to_string()],
        )
        .expect("stamp future version");
    }

    let out = env.cmd().args(["status"]).output().expect("spawn");
    // `tome status` maps the open-time SchemaTooNew error into exit 52
    // before it reaches the `OverallHealth` branch; the existing
    // `error.rs` mapping puts `SchemaTooNew` at 52, so the CLI exits 52
    // rather than the status-aggregate exit 1.
    assert_eq!(
        out.status.code(),
        Some(52),
        "expected exit 52 SchemaTooNew, got {:?}, stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}
