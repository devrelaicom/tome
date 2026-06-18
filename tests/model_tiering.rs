//! Integration tests for Phase p11 / model tiering migrations.

#[test]
fn v5_to_v6_preserves_vector_bytes_and_sets_small_profile() {
    use tome::index::migrations;

    // Build a v5 DB: vec0 virtual table for skill_embeddings, plus the minimal
    // surrounding schema needed for apply_pending to succeed.
    tome::index::vec_ext::register_globally().expect("register sqlite-vec");
    let mut conn = rusqlite::Connection::open_in_memory().expect("open");

    // Create minimal v5 schema (vec0 virtual table)
    conn.execute_batch(
        "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;
         INSERT INTO meta (key, value) VALUES ('schema_version', '5');
         CREATE TABLE workspaces (
            id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT UNIQUE NOT NULL,
            created_at INTEGER NOT NULL, last_used_at INTEGER NOT NULL);
         INSERT INTO workspaces (name, created_at, last_used_at) VALUES ('global', 0, 0);
         CREATE TABLE skills (
            id INTEGER PRIMARY KEY AUTOINCREMENT, catalog TEXT NOT NULL,
            plugin TEXT NOT NULL, name TEXT NOT NULL, kind TEXT NOT NULL DEFAULT 'skill',
            description TEXT NOT NULL, plugin_version TEXT NOT NULL, path TEXT NOT NULL,
            content_hash TEXT NOT NULL, searchable INTEGER NOT NULL DEFAULT 1,
            user_invocable INTEGER NOT NULL DEFAULT 0, when_to_use TEXT,
            indexed_at INTEGER NOT NULL);
         INSERT INTO skills (catalog, plugin, name, description, plugin_version, path, content_hash, indexed_at)
            VALUES ('cat', 'plug', 'sk', 'd', '1.0.0', 'skills/sk/SKILL.md', 'h', 0);
         CREATE TABLE workspace_skills (
            workspace_id INTEGER NOT NULL, skill_id INTEGER NOT NULL,
            enabled_at INTEGER NOT NULL, tier INTEGER NOT NULL DEFAULT 3,
            PRIMARY KEY (workspace_id, skill_id));
         INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at) VALUES (1, 1, 0);
         CREATE VIRTUAL TABLE skill_embeddings USING vec0(
            skill_id INTEGER PRIMARY KEY,
            embedding FLOAT[384]);",
    )
    .expect("create v5 schema");

    // Build a known 384-d f32 vector with recognizable byte pattern
    let known_vec: Vec<f32> = (0..384).map(|i| i as f32 * 0.001).collect();
    let known_blob: Vec<u8> = known_vec
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect();

    // Insert using the vec0 API (INSERT with the raw bytes)
    conn.execute(
        "INSERT INTO skill_embeddings (skill_id, embedding) VALUES (1, ?1)",
        rusqlite::params![known_blob],
    )
    .expect("insert embedding into vec0");

    // Run the migration
    let new_version = migrations::apply_pending(&mut conn, 5, 6).expect("migration");
    assert_eq!(new_version, 6);

    // Verify bytes preserved
    let got: Vec<u8> = conn
        .query_row(
            "SELECT embedding FROM skill_embeddings WHERE skill_id=1",
            [],
            |r| r.get(0),
        )
        .expect("read migrated embedding");
    assert_eq!(got, known_blob, "v6 must preserve the exact f32-LE bytes");

    // Verify profile stamped
    let profile: String = conn
        .query_row(
            "SELECT value FROM meta WHERE key='model_profile'",
            [],
            |r| r.get(0),
        )
        .expect("read model_profile");
    assert_eq!(profile, "small");
}
