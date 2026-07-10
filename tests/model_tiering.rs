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
    let known_blob: Vec<u8> = known_vec.iter().flat_map(|f| f.to_le_bytes()).collect();

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
    use tome::embedding::registry::{ModelKind, lookup};
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
        let entry =
            lookup(name).unwrap_or_else(|| panic!("entry `{name}` must be in MODEL_REGISTRY"));

        // download_model creates <models_root>/<name>/ internally
        download_model(entry, models_root, None)
            .unwrap_or_else(|e| panic!("download `{name}` failed: {e}"));

        let model_dir = models_root.join(name);
        match entry.kind {
            ModelKind::Embedder => {
                let embedder = FastembedEmbedder::load(entry, &model_dir)
                    .unwrap_or_else(|e| panic!("load embedder `{name}` failed: {e}"));
                let result = embedder
                    .embed("hello world")
                    .unwrap_or_else(|e| panic!("embed `{name}` failed: {e}"));
                let expected_dim = entry
                    .embedding_dim
                    .expect("embedder must have embedding_dim")
                    as usize;
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
                let candidates = vec![Candidate {
                    skill_id: 1,
                    catalog: "c".to_owned(),
                    plugin: "p".to_owned(),
                    name: "n".to_owned(),
                    kind: EntryKind::Skill,
                    description: "test candidate".to_owned(),
                    plugin_version: "1.0.0".to_owned(),
                    path: "p".to_owned(),
                    distance: 0.1,
                }];
                reranker
                    .rerank("hello world", candidates)
                    .unwrap_or_else(|e| panic!("rerank `{name}` failed: {e}"));
            }
            ModelKind::Summariser => {}
        }
    }
}

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
//   3. a plain `plugin enable` AND a `catalog update` (driven through the real
//      `catalog::update::run` entry point — `enable::run` is a private module,
//      see the (3a) note) are both REFUSED by the shared B3 drift guard with a
//      `tome reindex` hint (no vector written); a SCOPED reindex is refused too
//      (B1, exit 47);
//   4. a whole-index `tome reindex` force-re-embeds EVERY row to 768-d and
//      stamps + clears the drift (B1);
//   5. a subsequent query SUCCEEDS — no dimension-mismatch error — proving the
//      index is internally consistent again.
// ===========================================================================

use crate::common::{
    HomeGuard, config_with_catalog, copy_sample_plugin_catalog, enrol_catalog_symlinked,
    fabricate_models, lifecycle_paths, stub_reranker_seed, stub_summariser_seed,
};
use tempfile::TempDir;
use tome::cli::CatalogUpdateArgs;
use tome::commands::catalog::update as catalog_update;
use tome::commands::reindex::{self, Scope};
use tome::embedding::Embedder;
use tome::embedding::stub::StubEmbedder;
use tome::index::meta::{self, MetaKey, ModelIdent};
use tome::index::{self, MetaSeed, OpenOptions};
use tome::output::Mode;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{ResolvedScope, Scope as WsScope, ScopeSource, WorkspaceName};

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
            embedder: MetaSeed {
                name: MEDIUM_EMBEDDER.into(),
                version: "1.5".into(),
            },
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
            profile: None,
        },
    )
    .expect("open index")
}

#[test]
fn mixed_dimension_profile_switch_is_refused_until_reindex() {
    let tmp = TempDir::new().unwrap();

    // Root the fixture at `<home>/.tome` so a production `*::run` entry point —
    // which resolves `Paths` from `$HOME/.tome` via `Paths::resolve` — lands on
    // the SAME on-disk index/catalogs the explicit-`paths` helpers below build.
    // `HomeGuard` redirects `$HOME` under a process-global mutex and restores it
    // on drop; step (3b) relies on this to drive the real `catalog update`.
    let home = tmp.path().to_path_buf();
    let _home_guard = HomeGuard::install(&home);
    let paths = lifecycle_paths(&home.join(".tome"));
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
            embedder_seed: MetaSeed {
                name: MEDIUM_EMBEDDER.into(),
                version: "1.5".into(),
            },
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
        &ModelIdent {
            name: MEDIUM_EMBEDDER.into(),
            version: "1.5".into(),
        },
    )
    .expect("no drift in the matched baseline");

    let blob_len: i64 = conn
        .query_row(
            "SELECT LENGTH(embedding) FROM skill_embeddings LIMIT 1",
            [],
            |r| r.get(0),
        )
        .expect("read one stored embedding length");
    assert_eq!(
        blob_len,
        384 * 4,
        "stored vectors are 384-d (1536 LE bytes)"
    );
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
    // `commands::plugin::enable::run` calls `guard_embedder_drift` (enable.rs
    // ~:53) before loading any model. That `run` lives in a PRIVATE module
    // (`mod enable;` in `commands/plugin/mod.rs`), so it is not reachable from
    // an integration test — we assert the shared guard at the helper level
    // here and exercise the SIBLING production call site (`catalog update`)
    // through its public `run` in (3b) below, keeping at least one real entry
    // point on the test's critical path.
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

    // ---- (3b) a `catalog update` is REFUSED via the REAL command -------
    // Drive the production `commands::catalog::update::run` (public `run`),
    // NOT the guard helper directly. `run` resolves `Paths` from `$HOME/.tome`
    // (redirected to this fixture by `HomeGuard` above), opens the index, and
    // calls `guard_embedder_drift` at `commands/catalog/update.rs` ~:57 BEFORE
    // any git fetch or model load — so the drift state stamped in `meta` makes
    // it refuse before touching the network. This keeps the guard CALL SITE on
    // the test's critical path: deleting that guard call would fail this test.
    let update_scope = ResolvedScope {
        scope: WsScope(WorkspaceName::global()),
        source: ScopeSource::GlobalFallback,
        project_root: None,
        overridden_project_marker: None,
    };
    let update_err =
        catalog_update::run(CatalogUpdateArgs { name: None }, &update_scope, Mode::Json)
            .expect_err("`catalog update` must refuse under embedder drift");
    assert_eq!(
        update_err.exit_code(),
        41,
        "the real `catalog update` refusal is EmbedderNameDrift (exit 41)",
    );
    assert!(
        update_err.to_string().to_lowercase().contains("reindex"),
        "the real refusal must direct the user to `tome reindex`: {update_err}",
    );

    // ---- (3c) a SCOPED reindex is ALSO refused (B1) --------------------
    // Re-embedding only one plugin while stamping the GLOBAL meta would leave
    // out-of-scope rows at the old dimension — the exact corruption. Only a
    // whole-index reindex may switch the embedder.
    //
    // Issue #316 note: `whole_index = false` is EXACTLY what any explicit #316
    // selection resolves to — a positional `<catalog>`, a `--catalog`, a
    // `--plugin`, or a glob (even one that happens to cover every plugin). So
    // this assertion pins the invariant that a scoped-via-new-flags reindex is
    // refused under embedder drift with exit 47 (and, since the policy returns
    // Err, never reaches `stamp_embedder_after_whole_index`). See the dedicated
    // `explicit_selection_is_never_whole_index` test below for the resolver side.
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
    let new_ident = ModelIdent {
        name: LARGE_EMBEDDER.into(),
        version: configured.version.to_owned(),
    };

    let conn = open_writable(&paths);
    let effective_force = reindex::embedder_change_policy(
        &conn, /* whole_index = */ true, /* args_force = */ false, &new_ident,
    )
    .expect("whole-index reindex under drift is allowed");
    assert!(
        effective_force,
        "embedder change must force a full re-embed"
    );
    drop(conn);

    let deps = LifecycleDeps {
        paths: &paths,
        scope: &ws_scope,
        config: &config,
        embedder: &embedder_768,
        embedder_seed: MetaSeed {
            name: new_ident.name.clone(),
            version: new_ident.version.clone(),
        },
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };
    let agg = reindex::run_with_deps(
        Scope::All,
        std::slice::from_ref(&alpha),
        &deps,
        effective_force,
        Mode::Json,
    )
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

// ===========================================================================
// Issue #316 — an EXPLICIT selection (via the new `--catalog`/`--plugin` flags
// or a positional token) is NON-whole-index end-to-end: driven through the REAL
// `reindex::run` entry point, a `--catalog <c>` reindex under embedder drift is
// REFUSED with exit 47 and does NOT stamp the global `meta` embedder identity.
//
// This is the production-path complement to the resolver-side unit tests
// (`resolve_explicit` asserts `!whole_index`) and to step (3c) of the scenario
// above (which drives the policy gate directly). Here the FULL command runs:
// `run_inner` resolves the `--catalog` selection to `whole_index = false`, then
// `embedder_change_policy(whole_index = false, …)` refuses BEFORE any embedder
// model load — so the meta stamp is unreachable and the stored identity stays
// the OLD one.
// ===========================================================================

#[test]
fn explicit_catalog_selection_under_drift_is_refused_and_does_not_stamp() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().to_path_buf();
    let _home_guard = HomeGuard::install(&home);
    let paths = lifecycle_paths(&home.join(".tome"));
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    enrol_catalog_symlinked(&paths, "global", "sample-plugin-catalog", &catalog_root);

    let ws_scope = WsScope(WorkspaceName::global());
    let alpha: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();

    // Enable a plugin with a 384-d stub so the `--catalog` selection resolves to
    // a non-empty target set (it must not short-circuit to "Nothing to reindex").
    let embedder_384 = StubEmbedder::with_dim(384);
    {
        let deps = LifecycleDeps {
            paths: &paths,
            scope: &ws_scope,
            config: &config,
            embedder: &embedder_384,
            embedder_seed: MetaSeed {
                name: MEDIUM_EMBEDDER.into(),
                version: "1.5".into(),
            },
            reranker_seed: stub_reranker_seed(),
            summariser_seed: stub_summariser_seed(),
            allow_model_download: false,
        };
        lifecycle::enable(&alpha, &deps).expect("enable alpha with 384-d stub");
    }

    // Pin the baseline meta identity to the MEDIUM embedder, then flip the active
    // profile to `large` so the configured embedder drifts from the stored one.
    let conn = open_writable(&paths);
    meta::write(&conn, MetaKey::EmbedderName, MEDIUM_EMBEDDER).unwrap();
    meta::write(&conn, MetaKey::EmbedderVersion, "1.5").unwrap();
    meta::write(&conn, MetaKey::ModelProfile, "large").unwrap();
    drop(conn);

    // Drive the REAL `reindex::run` with an explicit `--catalog` selection. It
    // resolves to `whole_index = false`, so the drift policy refuses (exit 47)
    // before loading any embedder model.
    let scope = ResolvedScope {
        scope: WsScope(WorkspaceName::global()),
        source: ScopeSource::GlobalFallback,
        project_root: None,
        overridden_project_marker: None,
    };
    let err = reindex::run(
        tome::cli::ReindexArgs {
            scopes: Vec::new(),
            catalog: vec!["sample-plugin-catalog".to_owned()],
            plugin: Vec::new(),
            force: false,
        },
        &scope,
        Mode::Json,
    )
    .expect_err("an explicit --catalog selection under drift must be refused");
    assert_eq!(
        err.exit_code(),
        47,
        "explicit selection under embedder drift is ReindexScopedEmbedderChange (47)",
    );

    // The global meta embedder identity is UNCHANGED — the refusal fired before
    // `stamp_embedder_after_whole_index`, so no partial-stamp corruption.
    let conn = open_writable(&paths);
    let stored_name = meta::read(&conn, MetaKey::EmbedderName).unwrap().unwrap();
    assert_eq!(
        stored_name, MEDIUM_EMBEDDER,
        "a refused scoped reindex must NOT restamp the global embedder identity",
    );
}

// ===========================================================================
// Issue #498 — a WHOLE-INDEX reindex on an EMPTY index (no plugins enabled)
// adopts the active embedder as the baseline, clearing embedder drift.
//
// The catch-22 the fix resolves: change the embedder on a fresh index (no
// plugins yet) → the drift guard refuses `plugin enable` and directs the user
// to reindex, but a whole-index reindex on an empty corpus used to hit the
// "Nothing to reindex" early return BEFORE the `meta` embedder re-stamp — the
// sole drift resolver — so the drift never cleared and the first plugin could
// never be enabled.
//
// Both tests drive the REAL `reindex::run` entry point (NOT `run_with_deps`,
// which bypasses `run_inner`'s empty-corpus branch). There are zero enabled
// plugins, so the empty branch is exactly what runs.
// ===========================================================================

/// A `ResolvedScope` pointing at the privileged `global` workspace — the shape
/// `reindex::run` expects. Mirrors the inline value the drift refusal test
/// above builds.
fn global_resolved_scope() -> ResolvedScope {
    ResolvedScope {
        scope: WsScope(WorkspaceName::global()),
        source: ScopeSource::GlobalFallback,
        project_root: None,
        overridden_project_marker: None,
    }
}

#[test]
fn whole_index_reindex_on_empty_index_adopts_active_embedder_and_clears_drift() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().to_path_buf();
    let _home_guard = HomeGuard::install(&home);
    let paths = lifecycle_paths(&home.join(".tome"));
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    // Bootstrap an EMPTY index at the MEDIUM baseline (no plugin enabled → no
    // stored vectors), then flip the active profile to `large` so the configured
    // embedder drifts from the stored `meta` stamp — exactly the state a fresh
    // index lands in after an embedder change with nothing enabled yet.
    let conn = open_writable(&paths);
    meta::write(&conn, MetaKey::EmbedderName, MEDIUM_EMBEDDER).unwrap();
    meta::write(&conn, MetaKey::EmbedderVersion, "1.5").unwrap();
    meta::write(&conn, MetaKey::ModelProfile, "large").unwrap();

    // Sanity: the corpus is empty (no plugins enabled).
    let row_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM skill_embeddings", [], |r| r.get(0))
        .expect("count embeddings");
    assert_eq!(row_count, 0, "the index must be empty for this scenario");

    // Precondition — the catch-22: the drift guard (what `plugin enable` /
    // `catalog update` call) FIRES against the large-profile embedder, and the
    // whole-index reindex is the only resolver.
    let configured = meta::active_embedder(&conn).expect("resolve active embedder");
    assert_eq!(
        configured.name, LARGE_EMBEDDER,
        "the large profile resolves the large embedder",
    );
    let configured_ident = ModelIdent {
        name: LARGE_EMBEDDER.into(),
        version: configured.version.to_owned(),
    };
    let drift_err = meta::guard_embedder_drift(&conn, &configured_ident)
        .expect_err("drift must fire before the reindex resolves it");
    assert_eq!(
        drift_err.exit_code(),
        41,
        "stored medium vs configured large is EmbedderNameDrift (exit 41)",
    );
    drop(conn);

    // Run the REAL whole-index reindex (no scopes / --catalog / --plugin) on the
    // empty index. There is nothing to re-embed, but it must adopt the active
    // embedder as the baseline before returning "Nothing to reindex".
    reindex::run(
        tome::cli::ReindexArgs {
            scopes: Vec::new(),
            catalog: Vec::new(),
            plugin: Vec::new(),
            force: false,
        },
        &global_resolved_scope(),
        Mode::Json,
    )
    .expect("whole-index reindex on an empty index must succeed");

    // The global `meta` embedder identity is now the LARGE embedder — the stored
    // baseline matches the configured one, so the drift is cleared and the first
    // plugin can finally be enabled.
    let conn = open_writable(&paths);
    let stored_name = meta::read(&conn, MetaKey::EmbedderName).unwrap().unwrap();
    assert_eq!(
        stored_name, LARGE_EMBEDDER,
        "the empty whole-index reindex must adopt the active (large) embedder",
    );
    meta::guard_embedder_drift(&conn, &configured_ident)
        .expect("drift must be cleared after the empty whole-index reindex");
}

#[test]
fn scoped_empty_reindex_does_not_stamp_the_embedder_identity() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().to_path_buf();
    let _home_guard = HomeGuard::install(&home);
    let paths = lifecycle_paths(&home.join(".tome"));
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    // Enrol a catalog with NO enabled plugins so `--catalog <name>` resolves to
    // a VALID empty selection (`whole_index == false`, targets empty) — the
    // scoped-empty branch that must keep the "never stamp after a partial
    // re-embed" invariant.
    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    enrol_catalog_symlinked(&paths, "global", "sample-plugin-catalog", &catalog_root);

    // Same drift state as above: stored MEDIUM, active profile LARGE.
    let conn = open_writable(&paths);
    meta::write(&conn, MetaKey::EmbedderName, MEDIUM_EMBEDDER).unwrap();
    meta::write(&conn, MetaKey::EmbedderVersion, "1.5").unwrap();
    meta::write(&conn, MetaKey::ModelProfile, "large").unwrap();
    drop(conn);

    // A SCOPED empty selection short-circuits at the empty-corpus branch BEFORE
    // the policy gate (so it exits 0, not 47) and — crucially — does NOT stamp
    // the global `meta` identity, unlike the whole-index empty case.
    reindex::run(
        tome::cli::ReindexArgs {
            scopes: Vec::new(),
            catalog: vec!["sample-plugin-catalog".to_owned()],
            plugin: Vec::new(),
            force: false,
        },
        &global_resolved_scope(),
        Mode::Json,
    )
    .expect("a scoped empty reindex is a clean zero (exit 0)");

    // The stored embedder identity is UNCHANGED — a scoped selection never
    // adopts a new global baseline, so the drift is (correctly) still present.
    let conn = open_writable(&paths);
    let stored_name = meta::read(&conn, MetaKey::EmbedderName).unwrap().unwrap();
    assert_eq!(
        stored_name, MEDIUM_EMBEDDER,
        "a scoped empty reindex must NOT restamp the global embedder identity",
    );
    let configured = meta::active_embedder(&conn).expect("resolve active embedder");
    meta::guard_embedder_drift(
        &conn,
        &ModelIdent {
            name: configured.name.to_owned(),
            version: configured.version.to_owned(),
        },
    )
    .expect_err("drift must persist after a scoped empty reindex (no stamp)");
}

#[test]
fn whole_index_reindex_on_empty_index_persists_remote_dimension_and_clears_drift() {
    // The exact BYOK/BYOM scenario in #498's repro: an empty index whose active
    // embedder is a REMOTE provider (with a pinned `[embedding] dimensions`).
    // This exercises the REMOTE branch of `stamp_active_embedder_on_empty_index`
    // — the two current empty-index tests only cover the bundled path (where the
    // dimension key is DELETED). No HTTP happens: the empty branch never
    // constructs an embedder (nothing to embed), and `provider::resolve` reads
    // config only (no reachability check), so this is fully offline.
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().to_path_buf();
    let _home_guard = HomeGuard::install(&home);
    let paths = lifecycle_paths(&home.join(".tome"));
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    // Write a real `config.toml` pointing `[embedding]` at a remote OpenAI-
    // compatible provider with a pinned dimension — `reindex::run` reads this via
    // `config::load` and `provider::resolve`. `Paths::resolve` (used by `run`)
    // lands on `$HOME/.tome/config.toml` == `paths.global_config_file`.
    std::fs::write(
        &paths.global_config_file,
        "[embedding]\n\
         provider = \"myprov\"\n\
         model = \"text-embedding-3-small\"\n\
         dimensions = 1536\n\
         \n\
         [providers.myprov]\n\
         kind = \"openai\"\n\
         base_url = \"http://localhost:11434/v1\"\n\
         api_key = \"sk-test\"\n",
    )
    .unwrap();

    // Bootstrap an EMPTY index at the bundled MEDIUM baseline (no plugin enabled
    // → no stored vectors). The stored identity is the bundled `bge-base-en-v1.5`,
    // but the configured active embedder is now the remote provider → drift.
    let conn = open_writable(&paths);
    meta::write(&conn, MetaKey::EmbedderName, MEDIUM_EMBEDDER).unwrap();
    meta::write(&conn, MetaKey::EmbedderVersion, "1.5").unwrap();
    meta::write(&conn, MetaKey::ModelProfile, "medium").unwrap();
    let row_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM skill_embeddings", [], |r| r.get(0))
        .expect("count embeddings");
    assert_eq!(row_count, 0, "the index must be empty for this scenario");
    // No remote dimension is stamped yet.
    assert_eq!(
        meta::read_embedder_dimension(&conn).unwrap(),
        None,
        "no embedder_dimension before the remote reindex",
    );
    drop(conn);

    // Derive the EXPECTED remote identity via the same SSOT the production path
    // uses (`embedder_seed`), so the assertion stays coupled to actual behaviour
    // rather than a hardcoded version string.
    let cfg = tome::config::load(&paths).expect("load remote config");
    let expected_ident = {
        let conn = open_writable(&paths);
        let active = meta::active_embedder(&conn).expect("resolve active embedder");
        let seed = tome::embedding::embedder_seed(&cfg, active).expect("resolve remote seed");
        ModelIdent {
            name: seed.name,
            version: seed.version,
        }
    };
    assert_eq!(
        expected_ident.name, "myprov/text-embedding-3-small",
        "the remote embedder identity is `<provider>/<model>`",
    );

    // Precondition — the catch-22: the drift guard fires against the remote
    // identity, and the whole-index reindex is the only resolver.
    {
        let conn = open_writable(&paths);
        let drift_err = meta::guard_embedder_drift(&conn, &expected_ident)
            .expect_err("drift must fire before the reindex resolves it");
        assert_eq!(
            drift_err.exit_code(),
            41,
            "stored bundled vs configured remote is EmbedderNameDrift (exit 41)",
        );
    }

    // Run the REAL whole-index reindex on the empty index with the remote config.
    reindex::run(
        tome::cli::ReindexArgs {
            scopes: Vec::new(),
            catalog: Vec::new(),
            plugin: Vec::new(),
            force: false,
        },
        &global_resolved_scope(),
        Mode::Json,
    )
    .expect("whole-index reindex on an empty remote index must succeed");

    let conn = open_writable(&paths);
    // The global `meta` embedder identity is now the REMOTE embedder — drift is
    // cleared and the first plugin can finally be enabled.
    let stored_name = meta::read(&conn, MetaKey::EmbedderName).unwrap().unwrap();
    assert_eq!(
        stored_name, expected_ident.name,
        "the empty whole-index reindex must adopt the active (remote) embedder",
    );
    meta::guard_embedder_drift(&conn, &expected_ident)
        .expect("drift must be cleared after the empty remote whole-index reindex");

    // The pinned `[embedding] dimensions` is PERSISTED to `meta.embedder_dimension`
    // (the remote branch of the reconcile). This is what the bundled empty-index
    // tests do NOT cover — there the key is deleted.
    assert_eq!(
        meta::read_embedder_dimension(&conn).unwrap(),
        Some(1536),
        "a remote empty whole-index reindex persists the pinned [embedding] dimensions",
    );
}

// ===========================================================================
// Phase 12 / US4 review fix — corrupt-index self-heals to EXTINCTION on a
// bundled whole-index reindex.
//
// The MAJOR bug: a bundled `doctor --fix` runs `reindex --force` to repair a
// remote→bundled corrupt-index, but the reindex never cleared
// `meta.embedder_dimension`. So after the repair the stored vectors were
// bundled-dimension while `meta.embedder_dimension` still held the stale REMOTE
// value → `check_corrupt_index` re-surfaced the SAME finding on every run and it
// could never self-heal.
//
// This test proves the fix at the production reconcile path
// (`reindex::reconcile_embedder_dimension`, the function `run_inner` calls on a
// whole-index reindex):
//   1. stand up a real stub-embedded index (a 384-d corpus);
//   2. stamp `meta.embedder_dimension` to a WRONG (stale remote) value so
//      `check_corrupt_index` reports the finding;
//   3. run the BUNDLED whole-index reconcile (exactly as `run_inner` does);
//   4. assert `read_embedder_dimension == None` afterward AND
//      `check_corrupt_index` returns no finding — the finding is extinguished
//      and cannot re-surface, so a subsequent `doctor`/`doctor --fix` exits 0.
// ===========================================================================

#[test]
fn bundled_whole_index_reindex_extinguishes_corrupt_index_finding() {
    use tome::doctor::checks::check_corrupt_index;

    let tmp = TempDir::new().unwrap();
    let home = tmp.path().to_path_buf();
    let _home_guard = HomeGuard::install(&home);
    let paths = lifecycle_paths(&home.join(".tome"));
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    enrol_catalog_symlinked(&paths, "global", "sample-plugin-catalog", &catalog_root);

    let ws_scope = WsScope(WorkspaceName::global());
    let alpha: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();

    // (1) Enable a plugin with the 384-d stub so the index has real stored
    // vectors (LENGTH(embedding)/4 == 384).
    let embedder_384 = StubEmbedder::with_dim(384);
    {
        let deps = LifecycleDeps {
            paths: &paths,
            scope: &ws_scope,
            config: &config,
            embedder: &embedder_384,
            embedder_seed: MetaSeed {
                name: MEDIUM_EMBEDDER.into(),
                version: "1.5".into(),
            },
            reranker_seed: stub_reranker_seed(),
            summariser_seed: stub_summariser_seed(),
            allow_model_download: false,
        };
        lifecycle::enable(&alpha, &deps).expect("enable alpha with 384-d stub");
    }

    // (2) Stamp a WRONG (stale remote) `meta.embedder_dimension`. Stored vectors
    // are 384-d; pretend the index last reindexed against a 1024-d remote model.
    let conn = open_writable(&paths);
    meta::write_embedder_dimension(&conn, 1024).unwrap();
    assert_eq!(
        meta::read_embedder_dimension(&conn).unwrap(),
        Some(1024),
        "stale remote dimension is stamped",
    );
    drop(conn);

    // The finding is present: 384-d stored vs 1024-d meta.
    let finding = check_corrupt_index(&paths).expect("corrupt-index finding must be present");
    assert_eq!(finding.stored, 384);
    assert_eq!(finding.expected, 1024);

    // (3) Run the BUNDLED whole-index reconcile EXACTLY as `run_inner` does on a
    // bundled (`remote = false`) whole-index reindex. This is the production
    // function the bundled `doctor --fix` → `reindex --force` ultimately calls.
    let conn = open_writable(&paths);
    reindex::reconcile_embedder_dimension(
        &conn, /* remote = */ false, /* persisted_dim = */ None,
    )
    .expect("bundled whole-index reconcile clears the stale dimension");

    // (4a) The key is GONE — the stale remote dimension can no longer be read.
    assert_eq!(
        meta::read_embedder_dimension(&conn).unwrap(),
        None,
        "bundled reindex must clear meta.embedder_dimension",
    );
    drop(conn);

    // (4b) `check_corrupt_index` now reads "meta absent → N/A → no finding". The
    // finding is extinguished — it cannot re-surface on the next `doctor` run.
    assert_eq!(
        check_corrupt_index(&paths),
        None,
        "the corrupt-index finding must be extinguished after the bundled reindex \
         clears meta.embedder_dimension (self-heals to 0)",
    );
}

// ===========================================================================
// Task 9 — `tome models profile [show | set <tier>]` CLI surface.
//
// Driven through the compiled binary so the clap value_parser, the meta write,
// and the reindex/download notices are all exercised end-to-end.
// ===========================================================================

use crate::common::{ToolEnv, paths_for};
use serde_json::Value;

#[test]
fn models_profile_show_reports_default_medium_when_no_db_exists() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let out = env
        .cmd()
        .args(["--json", "models", "profile"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let rec: Value = serde_json::from_slice(&out.stdout).expect("--json object");
    assert_eq!(rec["profile"], "medium", "fresh install defaults to Medium");
    assert_eq!(rec["embedder"]["name"], "bge-base-en-v1.5");
    assert_eq!(rec["reranker"]["name"], "bge-reranker-large");
    // Each model line carries its install state (missing here, nothing fetched).
    assert_eq!(rec["embedder"]["state"], "missing");
}

#[test]
fn models_profile_set_writes_meta_and_show_reports_it() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let set = env
        .cmd()
        .args(["--json", "models", "profile", "small"])
        .output()
        .unwrap();
    assert!(
        set.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&set.stderr),
    );
    let set_rec: Value = serde_json::from_slice(&set.stdout).expect("set --json object");
    assert_eq!(set_rec["profile"], "small");
    assert_eq!(set_rec["embedder"], "bge-small-en-v1.5");
    assert_eq!(set_rec["reranker"], "bge-reranker-base");

    // `show` must now report `small` (persisted in meta.model_profile).
    let show = env
        .cmd()
        .args(["--json", "models", "profile"])
        .output()
        .unwrap();
    assert!(show.status.success());
    let show_rec: Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(show_rec["profile"], "small", "set must persist to meta");

    // The on-disk meta row is `small`.
    let conn = tome::index::open_read_only(&paths.index_db).expect("open index");
    let profile = tome::index::meta::active_profile(&conn).expect("active profile");
    assert_eq!(profile, tome::embedding::profile::Profile::Small);
}

#[test]
fn models_profile_set_large_from_medium_prints_reindex_notice() {
    // The DB bootstraps with the DEFAULT (medium) embedder stamped into
    // meta.embedder_name. Switching to `large` changes the embedder identity,
    // so the switch must surface the reindex notice (dim 768→1024).
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    // First touch creates the DB stamped with the medium embedder.
    let _ = env
        .cmd()
        .args(["models", "profile", "medium"])
        .output()
        .unwrap();

    // JSON proves the structured signal.
    let json = env
        .cmd()
        .args(["--json", "models", "profile", "large"])
        .output()
        .unwrap();
    assert!(json.status.success());
    let rec: Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(
        rec["embedder_changed"], true,
        "medium→large changes the embedder"
    );
    assert_eq!(rec["reindex_required"], true);
    assert_eq!(rec["prev_embedder_dim"], 768);
    assert_eq!(rec["new_embedder_dim"], 1024);

    // Human output names `reindex`.
    let human = env
        .cmd()
        .args(["models", "profile", "large"])
        .output()
        .unwrap();
    assert!(human.status.success());
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(
        text.contains("reindex"),
        "the embedder-change notice must mention `reindex`: {text}",
    );
    assert!(
        text.contains("768") && text.contains("1024"),
        "the notice must show the dimension change: {text}",
    );
}

#[test]
fn models_profile_set_rejects_invalid_tier_via_clap() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let out = env
        .cmd()
        .args(["models", "profile", "extra-large"])
        .output()
        .unwrap();
    assert!(!out.status.success(), "invalid tier must be rejected");
    assert_eq!(out.status.code(), Some(2), "clap usage error exits 2");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("small") && stderr.contains("medium") && stderr.contains("large"),
        "clap must list the valid tiers: {stderr}",
    );
}
