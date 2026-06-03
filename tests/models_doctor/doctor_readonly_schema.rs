//! Phase 7 / F-DOCTOR-RW — `tome doctor` (no `--fix`) read-only-schema
//! contract.
//!
//! `tome doctor` without `--fix` is a read-only pre-flight surface (FR-124 /
//! FR-002). Its per-subsystem checks MUST open the central index
//! READ-ONLY: they must NOT run forward schema migrations, must NOT take the
//! Tome advisory lock, and must DEGRADE (report what they can) rather than
//! abort on a stale OR a future schema. The offending surface is
//! [`doctor::checks::check_catalogs`], which opened with the migrating /
//! locking [`tome::index::open`].
//!
//! `tome doctor --fix` is the inverse: it DOES perform the lock-held
//! `repair_schema` migration. Test (c) pins that the `--fix` path still
//! migrates so the read-only fix does not regress the repair path.
//!
//! Stale-DB strategy: bootstrap a full current-schema DB via the production
//! `index::open`, seed one catalog enrolment, then surgically stamp
//! `meta.schema_version` to the target value. The v3→v4 migration is a
//! marker-only no-op (`phase6_kind_domain_agent_marker`), so a v4 DB stamped
//! down to v3 migrates cleanly back to v4 — exactly the silent-migration the
//! read-only path must NOT trigger.

use std::path::Path;

use crate::common::{ToolEnv, paths_for};
use tome::doctor::checks::check_catalogs;
use tome::index::{self, OpenOptions, SCHEMA_VERSION};
use tome::workspace::{Scope, WorkspaceName};

/// Bootstrap a full current-schema (v4) index DB and seed one `global`
/// enrolment so `check_catalogs` has a row to classify on the read path.
/// Returns the seeds matching what the CLI would re-open with (irrelevant
/// after bootstrap, but keeps the open call honest).
fn bootstrap_v_current_with_enrolment(paths: &tome::paths::Paths, catalog: &str, url: &str) {
    let opts = OpenOptions {
        embedder: crate::common::stub_embedder_seed(),
        reranker: crate::common::stub_reranker_seed(),
        summariser: crate::common::stub_summariser_seed(),
    };
    let conn = index::open(&paths.index_db, &opts).expect("bootstrap current-schema index db");
    index::workspace_catalogs::insert(&conn, "global", catalog, url, "main")
        .expect("seed enrolment");
}

/// Surgically stamp `meta.schema_version` to `version` via a short-lived
/// read-write connection that bypasses `index::open` (so no migration runs
/// during the stamp itself).
fn stamp_schema_version(path: &Path, version: u32) {
    let conn = rusqlite::Connection::open(path).expect("open db to stamp schema_version");
    let affected = conn
        .execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            rusqlite::params![version.to_string()],
        )
        .expect("stamp schema_version");
    assert_eq!(affected, 1, "schema_version row must exist to stamp");
}

/// Read `meta.schema_version` back via a fresh read-only connection — the
/// observation must not itself migrate.
fn read_schema_version(path: &Path) -> String {
    let conn = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .expect("open read-only to read schema_version");
    conn.query_row(
        "SELECT value FROM meta WHERE key = 'schema_version'",
        [],
        |row| row.get::<_, String>(0),
    )
    .expect("read schema_version")
}

fn global_scope() -> Scope {
    Scope(WorkspaceName::global())
}

// =====================================================================
// (a) STALE schema: read-only check completes, does NOT migrate, does
//     NOT take the advisory lock.
// =====================================================================

#[test]
fn check_catalogs_on_stale_schema_does_not_migrate_or_lock() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let url = "https://example.invalid/stale";
    bootstrap_v_current_with_enrolment(&paths, "stale-cat", url);

    // Stamp the on-disk schema one version behind the compiled version so a
    // forward migration WOULD fire if the check used `index::open`.
    let stale = SCHEMA_VERSION - 1;
    stamp_schema_version(&paths.index_db, stale);
    assert_eq!(
        read_schema_version(&paths.index_db),
        stale.to_string(),
        "precondition: DB is stamped at the stale version",
    );

    // The read-only doctor surface must complete without error.
    let out = check_catalogs(&paths, &global_scope()).expect("check_catalogs must not abort");
    assert_eq!(
        out.len(),
        1,
        "the seeded enrolment must still be classified"
    );
    assert_eq!(out[0].name, "stale-cat");

    // INVARIANT: no migration ran — the on-disk schema is UNCHANGED.
    assert_eq!(
        read_schema_version(&paths.index_db),
        stale.to_string(),
        "read-only doctor must NOT migrate an unlocked DB during a health check",
    );

    // INVARIANT: the advisory lock was never created/left behind.
    assert!(
        !paths.index_lock.exists(),
        "read-only doctor must NOT take the advisory lock (index.lock present)",
    );
}

// =====================================================================
// (b) FUTURE schema: read-only check DEGRADES (empty report), does NOT
//     abort with the exit-73 SchemaVersionTooNew.
// =====================================================================

#[test]
fn check_catalogs_on_future_schema_degrades_not_aborts() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let url = "https://example.invalid/future";
    bootstrap_v_current_with_enrolment(&paths, "future-cat", url);

    // Stamp the on-disk schema one version AHEAD of the compiled version.
    let future = SCHEMA_VERSION + 1;
    stamp_schema_version(&paths.index_db, future);

    // The read-only doctor surface must DEGRADE: a future schema can't be
    // read safely, so the open is swallowed into an empty/best-effort
    // result. It must NOT propagate SchemaVersionTooNew (exit 73) /
    // SchemaTooNew (exit 52).
    let out = check_catalogs(&paths, &global_scope())
        .expect("check_catalogs must degrade, not abort, on a future schema");
    assert!(
        out.is_empty(),
        "a future-schema DB the check cannot read must degrade to an empty enrolment list; got {out:?}",
    );

    // INVARIANT: still no migration, still no lock.
    assert_eq!(
        read_schema_version(&paths.index_db),
        future.to_string(),
        "read-only doctor must not rewrite schema_version on a future-schema DB",
    );
    assert!(
        !paths.index_lock.exists(),
        "read-only doctor must NOT take the advisory lock on a future-schema DB",
    );
}

// =====================================================================
// (c) `doctor --fix` on a STALE DB: the lock-held repair_schema migration
//     STILL runs (the read-only fix must not regress the repair path).
// =====================================================================

#[test]
fn doctor_fix_on_stale_schema_still_migrates() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    use tome::doctor::{self, Subsystem};

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    crate::common::fabricate_all_registry_models(&paths);

    let url = "https://example.invalid/fix";
    bootstrap_v_current_with_enrolment(&paths, "fix-cat", url);

    let home = tempfile::TempDir::new().unwrap();
    let scope = tome::workspace::ResolvedScope::global_fallback();

    // Assemble a valid report, then drive the `--fix` schema-repair branch
    // directly: stamp the DB to the stale version and queue the auto-fixable
    // `Subsystem::Schema` fix. This isolates `repair_schema`'s lock-held
    // migration independent of `assemble_report`'s internal check ordering.
    let mut report = doctor::assemble_report(&scope, &paths, home.path(), false).unwrap();

    let stale = SCHEMA_VERSION - 1;
    stamp_schema_version(&paths.index_db, stale);
    assert_eq!(
        read_schema_version(&paths.index_db),
        stale.to_string(),
        "precondition: DB is stamped at the stale version before --fix",
    );

    report.suggested_fixes.push(doctor::SuggestedFix {
        subsystem: Subsystem::Schema,
        diagnosis: "test: stale schema needs forward migration".to_owned(),
        command: "tome doctor --fix".to_owned(),
        auto_fixable: true,
    });

    let attempts = doctor::fixes::apply(
        &mut report,
        &doctor::fixes::FixContext {
            paths: &paths,
            scope: &scope,
            home: home.path(),
            force: false,
        },
    );
    assert!(attempts >= 1, "the queued Schema fix must be attempted");

    // INVARIANT: `--fix` performed the lock-held migration — the on-disk
    // schema advanced to the compiled version.
    assert_eq!(
        read_schema_version(&paths.index_db),
        SCHEMA_VERSION.to_string(),
        "doctor --fix must still perform the forward schema migration",
    );
}
