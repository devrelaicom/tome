//! Integration tests for Phase p11 / model tiering migrations.

mod common;

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

/// Network-gated smoke test: download each new model entry, load it, and run
/// one embed/rerank. Asserts that the output dimension matches `embedding_dim`
/// for embedders. Skipped in normal CI (`#[ignore]`); run manually with:
///
/// ```
/// cargo test --test model_tiering -- --ignored new_models_load_and_infer
/// ```
///
/// Expected: PASS (downloads ~450 MB total). This is the real validation that
/// the new ONNX graphs are CPU-safe in our `ort` stack.
#[test]
#[ignore]
fn new_models_load_and_infer() {
    use tome::embedding::download::download_model;
    use tome::embedding::fastembed::{FastembedEmbedder, FastembedReranker};
    use tome::embedding::registry::{lookup, ModelKind};
    use tome::embedding::{Embedder, Reranker};
    use tome::index::query::Candidate;
    use tome::plugin::identity::EntryKind;

    let new_model_names = &[
        "bge-base-en-v1.5",
        "bge-large-en-v1.5",
        "bge-reranker-large",
        "bge-reranker-v2-m3",
    ];

    let tmp = tempfile::tempdir().expect("tempdir");
    let models_root = tmp.path();

    for &name in new_model_names {
        let entry = lookup(name).unwrap_or_else(|| panic!("entry `{name}` must be in MODEL_REGISTRY"));

        // download_model creates <models_root>/<name>/ internally
        download_model(entry, models_root, None)
            .unwrap_or_else(|e| panic!("download `{name}` failed: {e}"));

        let model_dir = models_root.join(name);
        match entry.kind {
            ModelKind::Embedder => {
                let embedder = FastembedEmbedder::load(entry, &model_dir)
                    .unwrap_or_else(|e| panic!("load embedder `{name}` failed: {e}"));
                let result = embedder.embed("hello world")
                    .unwrap_or_else(|e| panic!("embed `{name}` failed: {e}"));
                let expected_dim = entry.embedding_dim.expect("embedder must have embedding_dim") as usize;
                assert_eq!(
                    result.len(),
                    expected_dim,
                    "embedder `{name}` output dim mismatch: got {} expected {}",
                    result.len(),
                    expected_dim,
                );
            }
            ModelKind::Reranker => {
                let reranker = FastembedReranker::load(entry, &model_dir)
                    .unwrap_or_else(|e| panic!("load reranker `{name}` failed: {e}"));
                let candidates = vec![
                    Candidate {
                        skill_id: 1,
                        catalog: "c".to_owned(),
                        plugin: "p".to_owned(),
                        name: "n".to_owned(),
                        kind: EntryKind::Skill,
                        description: "test candidate".to_owned(),
                        plugin_version: "1.0.0".to_owned(),
                        path: "p".to_owned(),
                        distance: 0.1,
                    },
                ];
                reranker.rerank("hello world", candidates)
                    .unwrap_or_else(|e| panic!("rerank `{name}` failed: {e}"));
            }
            ModelKind::Summariser => {}
        }
    }
}


// ===========================================================================
// Task 8 / S1 — deterministic mixed-dimension regression test (NON-network).
//
// The corruption B1/B3 prevent: a profile-switch that changes the embedder
// (and therefore the embedding DIMENSION) must NOT land new-dimension vectors
// in a table that still holds old-dimension vectors. The schema v6
// `skill_embeddings.embedding` is a dimension-free BLOB, so SQLite no longer
// rejects a mismatched-length vector at INSERT time — the guard is the only
// thing standing between a profile switch and a corrupt, un-queryable index.
//
// Scenario (mirrors the brief's Step 7), entirely model-free via StubEmbedder:
//   1. bootstrap + enable a plugin with a 384-d stub → meta stamps the stub
//      embedder identity, every row is a 384-d vector;
//   2. "install" a 768-d stub and flip `meta.model_profile` so the configured
//      active-profile embedder differs from the stored one (embedder DRIFT);
//   3. a plain `plugin enable` AND a `catalog update` are both REFUSED by the
//      shared B3 drift guard with a `tome reindex` hint (no vector written);
//   4. a whole-index `tome reindex` force-re-embeds EVERY row to 768-d and
//      clears the drift (B1);
//   5. a subsequent query SUCCEEDS — no dimension-mismatch error — proving the
//      index is internally consistent again.
// ===========================================================================
// Task 8 / S1 — deterministic mixed-dimension regression test (NON-network).
//
// The corruption B1/B3 prevent: a profile switch that changes the embedder
// (and therefore the embedding DIMENSION) must NOT land new-dimension vectors
// in a table that still holds old-dimension vectors. The schema v6
// `skill_embeddings.embedding` is a dimension-free BLOB, so SQLite no longer
// rejects a mismatched-length vector at INSERT time — the guard is the only
// thing standing between a profile switch and a corrupt, un-queryable index.
//
// Scenario (the brief's Step 7), entirely model-free via StubEmbedder:
//   1. enable a plugin with a 384-d stub against the MEDIUM (default) profile;
//      meta carries the medium embedder identity, every row is a 384-d vector;
//   2. flip `meta.model_profile` to `large` so the configured active-profile
//      embedder differs from the stored one (embedder NAME drift);
//   3. a plain `plugin enable` AND a `catalog update` are both REFUSED by the
//      shared B3 drift guard with a `tome reindex` hint (no vector written);
//      a SCOPED reindex is refused too (B1, exit 47);
//   4. a whole-index `tome reindex` force-re-embeds EVERY row to 768-d and
//      stamps + clears the drift (B1);
//   5. a subsequent query SUCCEEDS — no dimension-mismatch error — proving the
//      index is internally consistent again.

use crate::common::{
    config_with_catalog, copy_sample_plugin_catalog, enrol_catalog_symlinked, fabricate_models,
    lifecycle_paths, stub_reranker_seed, stub_summariser_seed,
};
use tempfile::TempDir;
use tome::commands::reindex::{self, Scope};
use tome::embedding::Embedder;
use tome::embedding::stub::StubEmbedder;
use tome::index::meta::{self, MetaKey, ModelIdent};
use tome::index::{self, MetaSeed, OpenOptions};
use tome::output::Mode;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{Scope as WsScope, WorkspaceName};

/// The MEDIUM-profile embedder identity stamped into `meta` for the baseline.
const MEDIUM_EMBEDDER: &str = "bge-base-en-v1.5";
/// The LARGE-profile embedder identity the `large` profile resolves to.
const LARGE_EMBEDDER: &str = "bge-large-en-v1.5";

fn open_writable(paths: &tome::paths::Paths) -> rusqlite::Connection {
    // `index::open` ignores `OpenOptions` on a re-open, so the seeds here only
    // matter on a first-touch bootstrap. The test stamps the `meta` identity
    // explicitly below, so these are placeholders.
    index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: MetaSeed { name: MEDIUM_EMBEDDER.into(), version: "1.5".into() },
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .expect("open index")
}

#[test]
fn mixed_dimension_profile_switch_is_refused_until_reindex() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    enrol_catalog_symlinked(&paths, "global", "sample-plugin-catalog", &catalog_root);

    let ws_scope = WsScope(WorkspaceName::global());
    let alpha: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();

    // ---- (1) enable with the 384-d stub (MEDIUM/default profile) --------
    let embedder_384 = StubEmbedder::with_dim(384);
    {
        let deps = LifecycleDeps {
            paths: &paths,
            scope: &ws_scope,
            config: &config,
            embedder: &embedder_384,
            embedder_seed: MetaSeed { name: MEDIUM_EMBEDDER.into(), version: "1.5".into() },
            reranker_seed: stub_reranker_seed(),
            summariser_seed: stub_summariser_seed(),
            allow_model_download: false,
        };
        lifecycle::enable(&alpha, &deps).expect("enable alpha with 384-d stub");
    }

    // Pin the baseline `meta` identity deterministically to the MEDIUM embedder
    // (the enrolment helper may have bootstrapped the DB before `enable`, and
    // `index::open` ignores reopen `OpenOptions`). The active profile is the
    // default (Medium) at this point, so the configured embedder MATCHES — no
    // drift yet — and every stored vector is 384-d.
    let conn = open_writable(&paths);
    meta::write(&conn, MetaKey::EmbedderName, MEDIUM_EMBEDDER).unwrap();
    meta::write(&conn, MetaKey::EmbedderVersion, "1.5").unwrap();
    meta::write(&conn, MetaKey::ModelProfile, "medium").unwrap();

    let configured_medium = meta::active_embedder(&conn).expect("resolve active embedder");
    assert_eq!(
        configured_medium.name, MEDIUM_EMBEDDER,
        "default/medium profile resolves the medium embedder",
    );
    meta::guard_embedder_drift(
        &conn,
        &ModelIdent { name: MEDIUM_EMBEDDER.into(), version: "1.5".into() },
    )
    .expect("no drift in the matched baseline");

    let blob_len: i64 = conn
        .query_row(
            "SELECT LENGTH(embedding) FROM skill_embeddings LIMIT 1",
            [],
            |r| r.get(0),
        )
        .expect("read one stored embedding length");
    assert_eq!(blob_len, 384 * 4, "stored vectors are 384-d (1536 LE bytes)");
    let row_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM skill_embeddings", [], |r| r.get(0))
        .unwrap();
    assert!(row_count >= 1, "plugin-alpha must have embedded rows");

    // ---- (2) profile switch: configured embedder now != stored ---------
    meta::write(&conn, MetaKey::ModelProfile, "large").unwrap();
    let configured = meta::active_embedder(&conn).expect("resolve active embedder");
    let configured_ident = ModelIdent {
        name: configured.name.to_owned(),
        version: configured.version.to_owned(),
    };
    assert_eq!(
        configured.name, LARGE_EMBEDDER,
        "the `large` profile selects the large embedder",
    );
    assert_ne!(
        configured.name, MEDIUM_EMBEDDER,
        "the switch must change the configured embedder",
    );

    // ---- (3a) a plain `plugin enable` is REFUSED -----------------------
    // `enable::run` calls `guard_embedder_drift` before loading any model.
    let enable_err = meta::guard_embedder_drift(&conn, &configured_ident)
        .expect_err("enable must refuse under embedder drift");
    assert_eq!(
        enable_err.exit_code(),
        41,
        "embedder NAME drift exits 41 (EmbedderNameDrift)",
    );
    assert!(
        enable_err.to_string().to_lowercase().contains("reindex"),
        "the refusal must direct the user to `tome reindex`: {enable_err}",
    );

    // ---- (3b) a `catalog update` is REFUSED the same way ---------------
    let update_err = meta::guard_embedder_drift(&conn, &configured_ident)
        .expect_err("catalog update must refuse under embedder drift");
    assert_eq!(update_err.exit_code(), 41);
    assert!(update_err.to_string().to_lowercase().contains("reindex"));

    // ---- (3c) a SCOPED reindex is ALSO refused (B1) --------------------
    // Re-embedding only one plugin while stamping the GLOBAL meta would leave
    // out-of-scope rows at the old dimension — the exact corruption. Only a
    // whole-index reindex may switch the embedder.
    let scoped_refusal = reindex::embedder_change_policy(
        &conn,
        /* whole_index = */ false,
        /* args_force = */ false,
        &configured_ident,
    );
    assert_eq!(
        scoped_refusal
            .expect_err("scoped reindex under embedder drift must be refused (B1)")
            .exit_code(),
        47,
        "scoped-embedder-change refusal has its own exit code (47)",
    );
    drop(conn);

    // ---- (4) whole-index reindex force-re-embeds to 768-d (B1) ---------
    // The 768-d stub stands in for the `large` profile's real embedder.
    let embedder_768 = StubEmbedder::with_dim(768);
    // The seed identity the new embedder writes into `meta` after re-embed —
    // the LARGE-profile embedder name (what `active_embedder` now resolves).
    let new_ident = ModelIdent { name: LARGE_EMBEDDER.into(), version: configured.version.to_owned() };

    let conn = open_writable(&paths);
    let effective_force = reindex::embedder_change_policy(
        &conn,
        /* whole_index = */ true,
        /* args_force = */ false,
        &new_ident,
    )
    .expect("whole-index reindex under drift is allowed");
    assert!(effective_force, "embedder change must force a full re-embed");
    drop(conn);

    let deps = LifecycleDeps {
        paths: &paths,
        scope: &ws_scope,
        config: &config,
        embedder: &embedder_768,
        embedder_seed: MetaSeed { name: new_ident.name.clone(), version: new_ident.version.clone() },
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };
    let agg = reindex::run_with_deps(Scope::All, std::slice::from_ref(&alpha), &deps, effective_force, Mode::Json)
        .expect("whole-index force reindex");
    assert!(agg.skills_re_embedded >= 1, "every row must be re-embedded");

    // Stamp the GLOBAL meta exactly as the whole-index reindex does post-commit.
    let conn = open_writable(&paths);
    reindex::stamp_embedder_after_whole_index(&conn, &new_ident)
        .expect("stamp meta after whole-index re-embed");

    // Drift is now cleared against the new (large) embedder.
    meta::guard_embedder_drift(&conn, &new_ident)
        .expect("drift must be cleared after a whole-index force re-embed");

    // Every stored vector is now 768-d (no mixed dimensions left behind).
    let lens: Vec<i64> = {
        let mut stmt = conn
            .prepare("SELECT LENGTH(embedding) FROM skill_embeddings")
            .unwrap();
        let v: Vec<i64> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        v
    };
    assert!(!lens.is_empty(), "rows still present after reindex");
    for len in &lens {
        assert_eq!(*len, 768 * 4, "every row must be re-embedded to 768-d");
    }
    drop(conn);

    // ---- (5) a query SUCCEEDS — no dimension-mismatch error ------------
    let query_conn = open_writable(&paths);
    let qvec = embedder_768
        .embed("alpha widget")
        .expect("embed query with the 768-d stub");
    let hits = tome::index::query::knn(
        &query_conn,
        "global",
        &qvec,
        10,
        &tome::index::query::QueryFilters::default(),
    )
    .expect("knn must succeed against a consistent 768-d index");
    assert!(
        !hits.is_empty(),
        "the consistent 768-d index must return hits for a 768-d query",
    );
}
