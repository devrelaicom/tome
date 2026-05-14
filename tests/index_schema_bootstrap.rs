//! Integration tests for the index database bootstrap pipeline (slice 4a).
//!
//! Each test creates a temporary directory, opens a fresh index database,
//! and asserts the on-disk shape matches the contract in
//! `contracts/index-schema.sql` plus the seed rows from research §R3.
//!
//! These tests are environment-clean: they pass an explicit path into
//! [`tome::index::open`] rather than mutating `HOME` / `XDG_DATA_HOME`.
//! See `specs/002-phase-2-plugins-index/retro/P2.md` for why env mutation
//! in the lib-test binary is off-limits.

use std::collections::HashSet;

use tempfile::TempDir;
use tome::index::{MetaSeed, OpenOptions, SCHEMA_VERSION, open};

fn options() -> OpenOptions {
    OpenOptions {
        embedder: MetaSeed {
            name: "stub-embedder".to_owned(),
            version: "0".to_owned(),
        },
        reranker: MetaSeed {
            name: "stub-reranker".to_owned(),
            version: "0".to_owned(),
        },
    }
}

fn db_path_in(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("index.db")
}

#[test]
fn fresh_open_bootstraps_schema_version() {
    let dir = TempDir::new().expect("tempdir");
    let conn = open(&db_path_in(&dir), &options()).expect("open should bootstrap");

    let version: String = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .expect("schema_version row");
    assert_eq!(version, SCHEMA_VERSION.to_string());
}

#[test]
fn fresh_open_creates_all_tables_and_indexes() {
    let dir = TempDir::new().expect("tempdir");
    let conn = open(&db_path_in(&dir), &options()).expect("open should bootstrap");

    let mut tables: HashSet<String> = HashSet::new();
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type IN ('table', 'index', 'view')")
        .expect("prepare sqlite_master");
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query sqlite_master");
    for r in rows {
        tables.insert(r.expect("row"));
    }

    for expected in [
        "meta",
        "skills",
        "skill_embeddings",
        "idx_skills_catalog_plugin",
        "idx_skills_enabled",
        "idx_skills_content_hash",
    ] {
        assert!(
            tables.contains(expected),
            "expected `{expected}` after bootstrap; got {tables:?}"
        );
    }
}

#[test]
fn fresh_open_seeds_meta_with_embedder_and_reranker() {
    let dir = TempDir::new().expect("tempdir");
    let conn = open(&db_path_in(&dir), &options()).expect("open should bootstrap");

    let pairs: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare("SELECT key, value FROM meta ORDER BY key")
            .expect("prepare meta");
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("query meta")
            .collect::<Result<_, _>>()
            .expect("rows")
    };

    let by_key: std::collections::HashMap<_, _> = pairs.into_iter().collect();
    assert_eq!(
        by_key.get("schema_version"),
        Some(&SCHEMA_VERSION.to_string())
    );
    assert_eq!(
        by_key.get("embedder_name"),
        Some(&"stub-embedder".to_owned())
    );
    assert_eq!(by_key.get("embedder_version"), Some(&"0".to_owned()));
    assert_eq!(
        by_key.get("reranker_name"),
        Some(&"stub-reranker".to_owned())
    );
    assert_eq!(by_key.get("reranker_version"), Some(&"0".to_owned()));
    assert!(
        by_key.contains_key("created_at"),
        "created_at must be seeded; got keys: {:?}",
        by_key.keys()
    );
}

#[test]
fn vec_extension_is_reachable() {
    let dir = TempDir::new().expect("tempdir");
    let conn = open(&db_path_in(&dir), &options()).expect("open should bootstrap");

    let version: String = conn
        .query_row("SELECT vec_version()", [], |row| row.get(0))
        .expect("vec_version() should succeed");
    assert!(
        version.starts_with('v') && version.contains('.'),
        "vec_version returned `{version}`; expected something like `v0.1.9`"
    );
}

#[test]
fn skill_embeddings_virtual_table_accepts_inserts() {
    // Defence in depth: the schema declared FLOAT[384]; this round-trips a
    // 384-dimensional zero vector to confirm the virtual table is usable.
    let dir = TempDir::new().expect("tempdir");
    let conn = open(&db_path_in(&dir), &options()).expect("open should bootstrap");

    // Insert a skill row first (foreign-key target).
    conn.execute(
        "INSERT INTO skills (catalog, plugin, name, description, plugin_version, path, content_hash, indexed_at) \
         VALUES ('cat', 'plug', 'sk', 'desc', '1.0.0', '/tmp/sk/SKILL.md', 'abc', '2026-05-12T00:00:00Z')",
        [],
    )
    .expect("insert skill");
    let skill_id: i64 = conn
        .query_row("SELECT last_insert_rowid()", [], |r| r.get(0))
        .expect("rowid");

    // 384 little-endian f32 zeros = 1536 bytes; sqlite-vec accepts blob form.
    let zeros: Vec<u8> = vec![0u8; 384 * std::mem::size_of::<f32>()];
    conn.execute(
        "INSERT INTO skill_embeddings (skill_id, embedding) VALUES (?1, ?2)",
        rusqlite::params![skill_id, zeros],
    )
    .expect("insert embedding");

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM skill_embeddings", [], |r| r.get(0))
        .expect("count");
    assert_eq!(count, 1);
}

#[test]
fn reopen_is_noop_on_already_bootstrapped_db() {
    let dir = TempDir::new().expect("tempdir");
    let path = db_path_in(&dir);
    let opts = options();

    {
        let _conn = open(&path, &opts).expect("first open bootstraps");
    }

    // Second open must succeed without re-running CREATE statements (which
    // would fail with "table already exists" if the bootstrap path fired).
    let conn = open(&path, &opts).expect("reopen should be no-op");
    let version: String = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .expect("schema_version still present");
    assert_eq!(version, SCHEMA_VERSION.to_string());
}

#[test]
fn schema_too_new_is_refused() {
    use tome::error::TomeError;

    let dir = TempDir::new().expect("tempdir");
    let path = db_path_in(&dir);
    let opts = options();

    {
        let conn = open(&path, &opts).expect("first open bootstraps");
        // Forcibly mark the stored schema as one higher than the compiled
        // binary understands. This simulates "user downgraded Tome".
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            rusqlite::params![(SCHEMA_VERSION + 1).to_string()],
        )
        .expect("bump version");
    }

    // Phase 3 / F7: the migration framework's refusal variant is
    // `SchemaVersionTooNew` (exit 73). The Phase 2 `SchemaTooNew` (exit
    // 52) still lives in the closed enum and is emitted by the read-only
    // open gate — see `open_read_only`'s docstring.
    let err = open(&path, &opts).expect_err("re-open must refuse newer schema");
    match err {
        TomeError::SchemaVersionTooNew { on_disk, expected } => {
            assert_eq!(on_disk, SCHEMA_VERSION + 1);
            assert_eq!(expected, SCHEMA_VERSION);
        }
        other => panic!("expected SchemaVersionTooNew, got {other:?}"),
    }
}
