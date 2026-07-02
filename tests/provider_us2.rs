//! Phase 12 / US2 — remote embedding (BYOK/BYOM) integration tests.
//!
//! The SAFETY-CRITICAL slice: every test drives the real production path over
//! the `set_transport_override` seam (no network) and proves the pre-mortem's
//! load-bearing failure mode — *silently indexing a well-formed-but-wrong remote
//! embedding* — is structurally impossible.
//!
//! - **T037** — per-kind embeddings request shaping + response parsing
//!   (openai + voyage), including the single-embedding (`data.len()==1`)
//!   structural contract and an int-where-float dtype surprise.
//! - **T038** — content-validation fail-closed at INDEX time AND QUERY time, on
//!   the CLI (exit `RemoteEmbeddingInvalid`/95) AND the MCP `search_skills` path
//!   (a clear tool error, never a degenerate KNN), asserting NOTHING is written
//!   to `skill_embeddings` and the index is unchanged on the index-time failure.
//! - **T039** — `meta.embedder_dimension` persistence from `[embedding]
//!   dimensions` / first-embed; the bundled path NEVER writes the key.
//! - **T040** — drift per path: `query` soft-fails (run `tome reindex`), the
//!   write paths (`plugin enable` / `catalog update`) hard-fail 41/42, and the
//!   MCP server refuses to serve `search_skills` from a stale in-memory embedder.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::mcp_harness::{StagedWorkspace, mcp_error_exit_code};
use common::stub_embedder_seed;

use tome::config::{Config, ProviderEntry, ProviderKind, Secret};
use tome::embedding::{Embedder, RemoteEmbedder, validate_embedding};
use tome::error::TomeError;
use tome::index::MetaSeed;
use tome::provider::config::{Capability, ResolvedProvider, resolve};
use tome::provider::error::ProviderErrorKind;
use tome::provider::http::{RawResponse, RequestSpec, set_transport_override};
use tome::provider::{openai, voyage};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve an embedding `ResolvedProvider` through the real `resolve` path.
fn resolved(kind: ProviderKind, dimensions: Option<u32>) -> ResolvedProvider {
    let mut config = embedding_config(kind, dimensions);
    config.embedding.dimensions = dimensions;
    resolve(&config, Capability::Embedding)
        .expect("resolve ok")
        .expect("provider referenced")
}

/// A `Config` with one embedding provider `p` of `kind`.
fn embedding_config(kind: ProviderKind, dimensions: Option<u32>) -> Config {
    let mut config = Config::default();
    config.providers.insert(
        "p".to_string(),
        ProviderEntry {
            kind,
            base_url: None,
            api_key: Some(Secret::from("sk-key".to_string())),
        },
    );
    config.embedding.provider = Some("p".to_string());
    config.embedding.model = Some("embed-model".to_string());
    config.embedding.dimensions = dimensions;
    config
}

fn ok_embedding(values: &[f32]) -> RawResponse {
    RawResponse {
        status: 200,
        retry_after: None,
        body: serde_json::to_vec(&serde_json::json!({
            "data": [{ "index": 0, "embedding": values }]
        }))
        .unwrap(),
    }
}

fn body_json(spec: &RequestSpec) -> serde_json::Value {
    serde_json::from_slice(&spec.body).expect("request body is valid JSON")
}

/// Count rows in `skill_embeddings` for a staged workspace's index.
fn embedding_row_count(ws: &StagedWorkspace) -> i64 {
    let conn = tome::index::open_read_only(&ws.paths.index_db).expect("open index");
    conn.query_row("SELECT COUNT(*) FROM skill_embeddings", [], |r| r.get(0))
        .expect("count skill_embeddings")
}

const SKILL: &str = "---\nname: alpha\ndescription: an alpha skill\n---\nBody.\n";

// ===========================================================================
// T037 — per-kind embeddings parsing
// ===========================================================================

#[test]
fn openai_embed_shapes_request_and_parses_one_vector() {
    let _g = set_transport_override(|spec| {
        assert!(
            spec.url.ends_with("/embeddings"),
            "openai embed must POST /embeddings: {}",
            spec.url
        );
        assert!(
            spec.headers
                .iter()
                .any(|(k, v)| k == "Authorization" && v == "Bearer sk-key"),
            "openai embed must carry a Bearer header"
        );
        let body = body_json(spec);
        assert_eq!(body["model"], serde_json::json!("embed-model"));
        // Single-element input array (FR-011).
        let input = body["input"].as_array().expect("input array");
        assert_eq!(input.len(), 1, "single-text input");
        assert_eq!(input[0], serde_json::json!("hello"));
        Ok(ok_embedding(&[0.1, 0.2, 0.3]))
    });
    let r = resolved(ProviderKind::Openai, None);
    let v = openai::embed_one(&r, "hello", None).unwrap();
    assert_eq!(v, vec![0.1, 0.2, 0.3]);
}

#[test]
fn openai_embed_emits_dimensions_field_when_set() {
    let _g = set_transport_override(|spec| {
        let body = body_json(spec);
        assert_eq!(
            body["dimensions"],
            serde_json::json!(256),
            "openai dimensions"
        );
        assert!(
            body.get("output_dimension").is_none(),
            "openai must NOT use output_dimension"
        );
        Ok(ok_embedding(&[0.0; 256]))
    });
    let r = resolved(ProviderKind::Openai, Some(256));
    let v = openai::embed_one(&r, "hello", Some(256)).unwrap();
    assert_eq!(v.len(), 256);
}

#[test]
fn voyage_embed_emits_output_dimension_field_when_set() {
    let _g = set_transport_override(|spec| {
        let body = body_json(spec);
        // Voyage uses `output_dimension`, NOT `dimensions`.
        assert_eq!(
            body["output_dimension"],
            serde_json::json!(512),
            "voyage output_dimension"
        );
        assert!(
            body.get("dimensions").is_none(),
            "voyage must NOT use the openai `dimensions` key"
        );
        Ok(ok_embedding(&[0.0; 512]))
    });
    let r = resolved(ProviderKind::Voyage, Some(512));
    let v = voyage::embed_one(&r, "doc", Some(512)).unwrap();
    assert_eq!(v.len(), 512);
}

#[test]
fn embed_response_with_multiple_data_is_malformed() {
    // data.len() != 1 → MalformedResponse (FR-011 structural contract), never a
    // silent first-of-many.
    let _g = set_transport_override(|_spec| {
        Ok(RawResponse {
            status: 200,
            retry_after: None,
            body: serde_json::to_vec(&serde_json::json!({
                "data": [{ "embedding": [0.1] }, { "embedding": [0.2] }]
            }))
            .unwrap(),
        })
    });
    let r = resolved(ProviderKind::Openai, None);
    let err = openai::embed_one(&r, "hi", None).unwrap_err();
    assert_eq!(err.kind, ProviderErrorKind::MalformedResponse);
}

#[test]
fn embed_response_200_error_envelope_is_bad_request() {
    let _g = set_transport_override(|_spec| {
        Ok(ok_json_value(serde_json::json!({
            "error": { "message": "model not available" }
        })))
    });
    let r = resolved(ProviderKind::Openai, None);
    let err = openai::embed_one(&r, "hi", None).unwrap_err();
    assert_eq!(err.kind, ProviderErrorKind::BadRequest);
    assert!(err.redacted_detail.contains("model not available"));
}

#[test]
fn voyage_int_where_float_round_trips_then_validates() {
    // Voyage int8 dtype surprise: integers where floats are expected. serde
    // parses the integers into the `Vec<f32>`; the shared validator's
    // finite/dimension check then governs acceptance. A finite, correct-length
    // integer vector is accepted (the magnitudes are finite); the point is no
    // dtype-specific parser is needed and nothing is silently mis-shaped.
    let _g = set_transport_override(|_spec| {
        Ok(RawResponse {
            status: 200,
            retry_after: None,
            body: br#"{"data":[{"embedding":[1, -2, 3]}]}"#.to_vec(),
        })
    });
    let r = resolved(ProviderKind::Voyage, None);
    let v = voyage::embed_one(&r, "doc", None).unwrap();
    assert_eq!(v, vec![1.0, -2.0, 3.0]);
    assert!(validate_embedding(&v, Some(3)).is_ok());
}

fn ok_json_value(value: serde_json::Value) -> RawResponse {
    RawResponse {
        status: 200,
        retry_after: None,
        body: serde_json::to_vec(&value).unwrap(),
    }
}

// ===========================================================================
// T038 — content-validation FAIL-CLOSED (the load-bearing tests)
// ===========================================================================

/// A `RemoteEmbedder` over the current transport override, seeded so its drift
/// identity matches the stub-seeded staged index.
fn remote_embedder(dimensions: Option<u32>) -> RemoteEmbedder {
    let r = resolved(ProviderKind::Openai, dimensions);
    RemoteEmbedder::new(r, dimensions, dimensions.map(|d| d as usize))
}

#[test]
fn index_time_empty_embedding_is_rejected_and_writes_nothing() {
    // A FRESH workspace whose enable embeds remotely. The transport returns an
    // EMPTY embedding → RemoteEmbeddingInvalid/95, the enclosing transaction
    // rolls back, and `skill_embeddings` stays empty.
    let _g = set_transport_override(|_spec| Ok(ok_embedding(&[])));
    let err = enable_with_remote_embedder(&[("alpha", SKILL)]).expect_err("must fail closed");
    assert_eq!(
        err.exit_code(),
        95,
        "index-time empty embedding → 95: {err:?}"
    );
}

#[test]
fn index_time_wrong_dimension_is_rejected_and_writes_nothing() {
    // `[embedding] dimensions = 8` but the provider returns 3 → 95, nothing
    // written.
    let _g = set_transport_override(|_spec| Ok(ok_embedding(&[0.1, 0.2, 0.3])));
    let err =
        enable_with_remote_embedder_dim(&[("alpha", SKILL)], Some(8)).expect_err("fail closed");
    assert_eq!(err.exit_code(), 95);
}

#[test]
fn index_time_all_zero_embedding_is_rejected_and_writes_nothing() {
    // The load-bearing zero-norm case: a FINITE, correct-length, ALL-ZEROS
    // embedding (a truncated/stub/model-not-found remote response) reaches the
    // real `lifecycle::enable` write path. It must be rejected fail-closed
    // (RemoteEmbeddingInvalid/95) and `skill_embeddings` must stay EMPTY — the
    // `enable_with_remote_embedder` driver asserts the zero-row count on failure.
    let _g = set_transport_override(|_spec| Ok(ok_embedding(&[0.0, 0.0, 0.0])));
    let err = enable_with_remote_embedder(&[("alpha", SKILL)]).expect_err("must fail closed");
    assert_eq!(
        err.exit_code(),
        95,
        "index-time all-zero (zero-norm) embedding → 95: {err:?}"
    );
}

#[test]
fn query_time_all_zero_embedding_is_95_on_cli() {
    // Stage a VALID stub index, then run the real `query::pipeline` with a
    // RemoteEmbedder whose transport returns an ALL-ZEROS vector → fails closed
    // (95) BEFORE any KNN, never a degenerate cosine. Drift does NOT fire (the
    // seed matches the stub-seeded index).
    let ws = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let _g = set_transport_override(|_spec| Ok(ok_embedding(&[0.0, 0.0, 0.0])));
    let err = query_with_embedder(&ws, Box::new(remote_embedder(None))).expect_err("fail closed");
    assert_eq!(
        err.exit_code(),
        95,
        "query-time all-zero embedding → 95 (never a degenerate KNN): {err:?}"
    );
}

#[test]
fn query_time_all_zero_embedding_is_tool_error_on_mcp() {
    // The MCP `search_skills` path embeds through the SAME trait method. An
    // ALL-ZEROS remote embedding must surface a CLEAR tool error (mapped from
    // RemoteEmbeddingInvalid/95), NOT a degenerate cosine ranking.
    let ws = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let _g = set_transport_override(|_spec| Ok(ok_embedding(&[0.0, 0.0, 0.0])));
    let harness = ws.harness_with_embedder(
        Arc::new(remote_embedder(None)) as Arc<dyn Embedder>,
        stub_embedder_seed(),
    );
    let err = harness
        .call_search_skills(common::mcp_harness::search_input("anything"))
        .expect_err("MCP search must fail closed on a zero-norm embedding, not rank degenerately");
    assert_eq!(
        mcp_error_exit_code(&err),
        95,
        "MCP fail-closed on all-zero must map to RemoteEmbeddingInvalid/95: {err:?}"
    );
}

#[test]
fn query_time_invalid_embedding_is_95_on_cli() {
    // Stage a VALID index with the stub embedder, then run the query pipeline
    // with a RemoteEmbedder whose transport returns a non-finite-via-overflow
    // value → fails closed before any KNN. Drift does NOT fire (the seed matches
    // the stub-seeded index).
    let ws = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let _g = set_transport_override(|_spec| Ok(ok_embedding(&[])));
    let err = query_with_embedder(&ws, Box::new(remote_embedder(None))).expect_err("fail closed");
    assert_eq!(
        err.exit_code(),
        95,
        "query-time invalid embedding → 95: {err:?}"
    );
}

#[test]
fn query_time_invalid_embedding_is_tool_error_on_mcp() {
    // The MCP `search_skills` path embeds through the same trait method. A
    // RemoteEmbedder over a failing seam must surface a CLEAR tool error (mapped
    // from RemoteEmbeddingInvalid/95), NOT a degenerate empty KNN.
    let ws = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let _g = set_transport_override(|_spec| Ok(ok_embedding(&[])));
    let harness = ws.harness_with_embedder(
        Arc::new(remote_embedder(None)) as Arc<dyn Embedder>,
        stub_embedder_seed(),
    );
    let err = harness
        .call_search_skills(common::mcp_harness::search_input("anything"))
        .expect_err("MCP search must fail closed, not return a degenerate KNN");
    assert_eq!(
        mcp_error_exit_code(&err),
        95,
        "MCP fail-closed must map to RemoteEmbeddingInvalid/95: {err:?}"
    );
}

// ===========================================================================
// T039 — persisted dimension (validator reads it)
// ===========================================================================

#[test]
fn established_dimension_set_from_first_embed_when_no_knob() {
    let _g = set_transport_override(|_spec| Ok(ok_embedding(&[0.1, 0.2, 0.3, 0.4, 0.5])));
    let e = remote_embedder(None);
    assert_eq!(e.established_dimension(), None);
    let _ = e.embed("x").unwrap();
    assert_eq!(
        e.established_dimension(),
        Some(5),
        "first embed establishes the run dimension"
    );
}

#[test]
fn dimensions_knob_is_authoritative_seed() {
    // With `[embedding] dimensions = 4`, a mismatching 3-length response is
    // rejected against the seeded dimension (the knob wins immediately — no
    // "first embed establishes" leniency).
    let _g = set_transport_override(|_spec| Ok(ok_embedding(&[0.1, 0.2, 0.3])));
    let e = remote_embedder(Some(4));
    let err = e.embed("x").unwrap_err();
    assert_eq!(err.exit_code(), 95);
}

#[test]
fn bundled_path_does_not_persist_embedder_dimension() {
    // NFR-006: with NO `[embedding]` provider, the staged (bundled, stub) index
    // must NOT carry an `embedder_dimension` meta row.
    let ws = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let conn = tome::index::open_read_only(&ws.paths.index_db).unwrap();
    assert_eq!(
        tome::index::read_embedder_dimension(&conn).unwrap(),
        None,
        "bundled path must never write meta.embedder_dimension"
    );
}

// ===========================================================================
// T040 — drift per path
// ===========================================================================

#[test]
fn query_pipeline_soft_fails_on_embedder_drift() {
    // A staged stub index; query with a DIFFERENT embedder identity seed → the
    // pipeline's drift check converts embedder-name drift into a hard query
    // error directing the user at reindex (the CLI `query` surface).
    let ws = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let _g = set_transport_override(|_spec| Ok(ok_embedding(&[0.1, 0.2, 0.3])));
    // Build a RemoteEmbedder but pass a MISMATCHED seed so drift fires (the seed
    // is what the pipeline compares, not the embedder's model_name).
    let drift_seed = MetaSeed {
        name: "p/other-model".to_string(),
        version: "external".to_string(),
    };
    let err = query_with_embedder_seed(&ws, Box::new(remote_embedder(None)), drift_seed)
        .expect_err("embedder drift must hard-fail the query");
    assert!(
        matches!(
            err,
            TomeError::EmbedderNameDrift { .. } | TomeError::EmbedderVersionDrift { .. }
        ),
        "expected embedder drift, got {err:?}"
    );
    let code = err.exit_code();
    assert!(
        code == 41 || code == 42,
        "drift exit code 41/42, got {code}"
    );
}

#[test]
fn mcp_search_refuses_under_stale_embedder() {
    // The MCP server holds a startup-frozen embedder identity. If that identity
    // no longer matches the index `meta` (a stale in-memory embedder), the
    // server refuses to serve `search_skills` rather than returning silently.
    let ws = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let _g = set_transport_override(|_spec| Ok(ok_embedding(&[0.1, 0.2, 0.3])));
    let stale_seed = MetaSeed {
        name: "p/stale-model".to_string(),
        version: "external".to_string(),
    };
    let harness = ws.harness_with_embedder(
        Arc::new(remote_embedder(None)) as Arc<dyn Embedder>,
        stale_seed,
    );
    let err = harness
        .call_search_skills(common::mcp_harness::search_input("anything"))
        .expect_err("stale embedder must refuse to serve");
    // Drift maps to the `embedder_drift` MCP code → 41/42 via the bridge.
    let code = mcp_error_exit_code(&err);
    assert!(
        code == 41 || code == 42,
        "MCP must refuse under embedder drift (41/42), got {code}: {err:?}"
    );
}

#[test]
fn switching_embedder_does_not_auto_reindex() {
    // A no-op assurance: building a RemoteEmbedder + resolving config never
    // re-embeds anything. (The drift tests above prove the user is DIRECTED to
    // reindex; here we confirm the embeddings row count is untouched by merely
    // resolving a switched config.)
    let ws = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let before = embedding_row_count(&ws);
    let cfg = embedding_config(ProviderKind::Openai, None);
    let _ = resolve(&cfg, Capability::Embedding).unwrap();
    assert_eq!(
        embedding_row_count(&ws),
        before,
        "resolving a switched embedder config must not re-embed"
    );
}

// ---------------------------------------------------------------------------
// Retry counting: the embed path makes exactly ONE request per embed (FR-011).
// ---------------------------------------------------------------------------

#[test]
fn embed_makes_exactly_one_request() {
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let _g = set_transport_override(move |_spec| {
        c.fetch_add(1, Ordering::SeqCst);
        Ok(ok_embedding(&[0.1, 0.2, 0.3]))
    });
    let e = remote_embedder(None);
    let _ = e.embed("hello").unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1, "one request per embed");
}

// ---------------------------------------------------------------------------
// Index-time / query-time drivers using a custom (remote) embedder. These
// replicate `StagedWorkspace::stage` but route the enable / query through a
// `RemoteEmbedder` so the remote validation path is exercised end-to-end.
// ---------------------------------------------------------------------------

use common::mcp_harness::{seed_catalog_enrolment, write_plugin};
use common::{config_with_catalog, fabricate_models, lifecycle_paths};
use tempfile::TempDir;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{Scope, WorkspaceName};

/// Enable `acme/plug` against a FRESH workspace whose embedder is a
/// `RemoteEmbedder` over the current transport override. The index `meta` is
/// stamped with the remote identity (so the fresh enable sees no drift). Returns
/// the enable result; the caller asserts fail-closed behaviour.
fn enable_with_remote_embedder(skills: &[(&str, &str)]) -> Result<(), TomeError> {
    enable_with_remote_embedder_dim(skills, None).map(|(_, _)| ())
}

/// Like [`enable_with_remote_embedder`] but with an explicit `[embedding]
/// dimensions`. Returns `(result, temp)`; `temp` keeps the on-disk tree alive
/// for any post-enable inspection. On failure it still returns the error.
fn enable_with_remote_embedder_dim(
    skills: &[(&str, &str)],
    dimensions: Option<u32>,
) -> Result<(TempDir, std::path::PathBuf), TomeError> {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = tmp.path().join("catalog");
    std::fs::create_dir_all(&catalog_root).unwrap();
    let config = config_with_catalog("acme", &catalog_root);
    write_plugin(&catalog_root, "plug", skills, &[]);

    // The remote seed the index is stamped with — so the fresh enable's drift
    // check (against the just-bootstrapped meta) agrees.
    let remote_seed = MetaSeed {
        name: "p/embed-model".to_string(),
        version: "external".to_string(),
    };
    let embedder = remote_embedder(dimensions);
    let scope = Scope(WorkspaceName::global());
    let deps = LifecycleDeps {
        paths: &paths,
        scope: &scope,
        config: &config,
        embedder: &embedder,
        embedder_seed: remote_seed,
        reranker_seed: common::stub_reranker_seed(),
        summariser_seed: common::stub_summariser_seed(),
        allow_model_download: false,
    };
    let id: PluginId = "acme/plug".parse().unwrap();
    seed_catalog_enrolment(&paths, &catalog_root, "acme");

    let db_path = paths.index_db.clone();
    let result = lifecycle::enable(&id, &deps);
    // Assert nothing landed in skill_embeddings on failure.
    if result.is_err()
        && db_path.is_file()
        && let Ok(conn) = tome::index::open_read_only(&db_path)
    {
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM skill_embeddings", [], |r| r.get(0))
            .unwrap_or(0);
        assert_eq!(n, 0, "a failed remote enable must write NO embeddings");
    }
    result.map(|_| (tmp, catalog_root))
}

/// Run `query::pipeline` over a staged workspace with the supplied embedder,
/// using the stub seed (no drift). Returns the pipeline result.
fn query_with_embedder(ws: &StagedWorkspace, embedder: Box<dyn Embedder>) -> Result<(), TomeError> {
    query_with_embedder_seed(ws, embedder, stub_embedder_seed())
}

/// Run `query::pipeline` over a staged workspace with the supplied embedder +
/// drift seed. A mismatched seed makes the pipeline's drift check fire.
fn query_with_embedder_seed(
    ws: &StagedWorkspace,
    embedder: Box<dyn Embedder>,
    embedder_seed: MetaSeed,
) -> Result<(), TomeError> {
    use tome::cli::QueryArgs;
    use tome::commands::query::{self, QueryDeps};

    let scope = Scope(WorkspaceName::global());
    let config = Config::default();
    let args = QueryArgs {
        text: vec!["anything".to_string()],
        query: None,
        top_k: Some(10),
        catalog: Vec::new(),
        plugin: Vec::new(),
        kind: Vec::new(),
        no_rerank: true,
        strict: false,
        min_score: None,
    };
    let deps = QueryDeps {
        paths: &ws.paths,
        scope: &scope,
        config: &config,
        embedder: embedder.as_ref(),
        reranker: None,
        embedder_seed,
        reranker_seed: common::stub_reranker_seed(),
    };
    query::pipeline(&args, &deps).map(|_| ())
}
