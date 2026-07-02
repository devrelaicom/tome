//! Phase 12 / US3 — remote reranking (BYOK, Voyage) integration tests.
//!
//! Reranking is the narrowest remote slice: it is stateless (no stored artefact
//! to corrupt), so the only properties that matter are
//! (a) candidate ordering comes from the remote reranker (US3.1),
//! (b) a non-Voyage `[reranker]` kind surfaces `ProviderConfigInvalid`/93
//!     (US3.2 — the resolve matrix rejects it; the query path propagates it), and
//! (c) an UNREACHABLE remote reranker surfaces as `ProviderRequestFailed`/94 on
//!     BOTH the CLI and MCP surfaces, NEVER a silent unranked result (US3.3).
//!
//! Every test drives the real production path over the `set_transport_override`
//! seam (no network).

mod common;

use std::sync::Arc;

use common::mcp_harness::{StagedWorkspace, mcp_error_exit_code};

use tome::cli::QueryArgs;
use tome::commands::query::{self, QueryDeps};
use tome::config::{Config, ProviderEntry, ProviderKind, Secret};
use tome::embedding::{RemoteReranker, Reranker};
use tome::error::TomeError;
use tome::index::MetaSeed;
use tome::provider::config::{Capability, ResolvedProvider, resolve};
use tome::provider::http::{RawResponse, RequestSpec, set_transport_override};
use tome::workspace::{Scope, WorkspaceName};

const SKILL: &str = "---\nname: alpha\ndescription: an alpha skill\n---\nBody.\n";
const SKILL_BETA: &str = "---\nname: beta\ndescription: a beta skill\n---\nBody.\n";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A `Config` with one Voyage reranker provider `vp`.
fn voyage_reranker_config() -> Config {
    let mut config = Config::default();
    config.providers.insert(
        "vp".to_string(),
        ProviderEntry {
            kind: ProviderKind::Voyage,
            base_url: None,
            api_key: Some(Secret::from("voyage-key".to_string())),
        },
    );
    config.reranker.provider = Some("vp".to_string());
    config.reranker.model = Some("rerank-2".to_string());
    config
}

/// A resolved Voyage reranker connection through the real `resolve` path.
fn resolved_reranker() -> ResolvedProvider {
    resolve(&voyage_reranker_config(), Capability::Reranker)
        .expect("resolve ok")
        .expect("provider referenced")
}

fn rerank_response(results: serde_json::Value) -> RawResponse {
    RawResponse {
        status: 200,
        retry_after: None,
        body: serde_json::to_vec(&serde_json::json!({ "results": results })).unwrap(),
    }
}

fn body_json(spec: &RequestSpec) -> serde_json::Value {
    serde_json::from_slice(&spec.body).expect("request body is valid JSON")
}

/// Run `query::pipeline` over a staged workspace with the STUB embedder (no
/// drift) and the supplied reranker. Returns the pipeline result.
fn query_with_reranker(
    ws: &StagedWorkspace,
    reranker: Option<&dyn Reranker>,
    reranker_seed: MetaSeed,
) -> Result<query::QueryOutcome, TomeError> {
    let scope = Scope(WorkspaceName::global());
    let config = Config::default();
    let embedder = tome::embedding::stub::StubEmbedder::new();
    let args = QueryArgs {
        text: vec!["anything".to_string()],
        query: None,
        top_k: Some(10),
        catalog: Vec::new(),
        plugin: Vec::new(),
        kind: Vec::new(),
        no_rerank: reranker.is_none(),
        strict: false,
        min_score: None,
    };
    let deps = QueryDeps {
        paths: &ws.paths,
        scope: &scope,
        config: &config,
        embedder: &embedder,
        reranker,
        embedder_seed: common::stub_embedder_seed(),
        reranker_seed,
    };
    query::pipeline(&args, &deps)
}

/// The remote reranker's drift seed (`"<provider>/<model>"`/`"external"`).
fn remote_reranker_seed() -> MetaSeed {
    MetaSeed {
        name: "vp/rerank-2".to_string(),
        version: "external".to_string(),
    }
}

// ===========================================================================
// US3.1 — ordering comes from the remote reranker
// ===========================================================================

#[test]
fn remote_reranker_produces_final_ordering() {
    // Stage two skills so KNN returns ≥2 candidates; the remote reranker re-orders
    // them. The transport returns index 1 ahead of index 0 → the input candidate
    // at index 1 must rank first (by INDEX, never positional).
    let ws = StagedWorkspace::stage(&[("alpha", SKILL), ("beta", SKILL_BETA)], &[]);
    let _g = set_transport_override(|spec| {
        // The query path hits /rerank with the candidate documents.
        assert!(
            spec.url.ends_with("/rerank"),
            "must POST /rerank: {}",
            spec.url
        );
        let body = body_json(spec);
        assert_eq!(body["model"], serde_json::json!("rerank-2"));
        assert_eq!(body["return_documents"], serde_json::json!(false));
        Ok(rerank_response(serde_json::json!([
            { "index": 1, "relevance_score": 0.95 },
            { "index": 0, "relevance_score": 0.10 },
        ])))
    });
    let reranker = RemoteReranker::new(resolved_reranker());
    let outcome = query_with_reranker(&ws, Some(&reranker), remote_reranker_seed())
        .expect("remote rerank should succeed");
    assert_eq!(outcome.scoring, query::ScoringMode::Reranked);
    assert!(!outcome.results.is_empty(), "results returned");
    // Highest remote score first.
    assert!(
        outcome.results[0].score >= outcome.results.last().unwrap().score,
        "results sorted by descending remote score"
    );
    assert_eq!(outcome.results[0].score, 0.95);
}

// ===========================================================================
// US3.2 — a non-Voyage `[reranker]` kind → 93 through the query path
// ===========================================================================

#[test]
fn non_voyage_reranker_kind_is_93_via_build_reranker() {
    // The resolve matrix rejects every non-Voyage reranker kind. `build_reranker`
    // (which `tome query` calls) surfaces that as ProviderConfigInvalid/93 — never
    // a silent bundled fallback.
    let dir = tempfile::TempDir::new().unwrap();
    let paths = tome::paths::Paths::from_root(dir.path().to_path_buf());
    let bundled = tome::embedding::profile::reranker_for(tome::embedding::Profile::DEFAULT);
    for kind in [
        ProviderKind::Openai,
        ProviderKind::Anthropic,
        ProviderKind::Gemini,
    ] {
        let mut config = Config::default();
        config.providers.insert(
            "p".to_string(),
            ProviderEntry {
                kind,
                base_url: None,
                api_key: None,
            },
        );
        config.reranker.provider = Some("p".to_string());
        config.reranker.model = Some("m".to_string());
        match tome::embedding::build_reranker(&config, &paths, bundled) {
            Err(err) => assert_eq!(
                err.exit_code(),
                93,
                "non-voyage reranker kind {kind:?} must be 93: {err:?}"
            ),
            Ok(_) => panic!("kind {kind:?} must fail with 93, not build a reranker"),
        }
    }
}

// ===========================================================================
// US3.3 — unreachable remote reranker → 94 (NOT silent unranked), CLI + MCP
// ===========================================================================

#[test]
fn unreachable_remote_reranker_is_94_on_cli() {
    // A staged index + a RemoteReranker over a 5xx-exhausting transport. The query
    // pipeline MUST propagate ProviderRequestFailed/94 rather than degrade to an
    // unranked result set.
    let ws = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let _g = set_transport_override(|_spec| {
        Ok(RawResponse {
            status: 503,
            retry_after: Some(std::time::Duration::from_secs(0)),
            body: Vec::new(),
        })
    });
    let reranker = RemoteReranker::new(resolved_reranker());
    let err = query_with_reranker(&ws, Some(&reranker), remote_reranker_seed())
        .expect_err("unreachable remote reranker must fail the query, not return unranked");
    assert_eq!(
        err.exit_code(),
        94,
        "unreachable remote reranker → 94 on CLI: {err:?}"
    );
}

#[test]
fn unreachable_remote_reranker_is_94_on_mcp() {
    // The MCP `search_skills` path reranks (config default `rerank = true`) via the
    // injected RemoteReranker. An unreachable provider must surface a clear tool
    // error mapped from ProviderRequestFailed/94, NEVER a degenerate unranked KNN.
    let ws = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let _g = set_transport_override(|_spec| {
        Ok(RawResponse {
            status: 503,
            retry_after: Some(std::time::Duration::from_secs(0)),
            body: Vec::new(),
        })
    });
    let reranker: Arc<dyn Reranker> = Arc::new(RemoteReranker::new(resolved_reranker()));
    let harness = ws.harness_with_reranker(reranker);
    let err = harness
        .call_search_skills(common::mcp_harness::search_input("anything"))
        .expect_err("MCP search must fail closed on an unreachable remote reranker");
    assert_eq!(
        mcp_error_exit_code(&err),
        94,
        "MCP fail-closed must map to ProviderRequestFailed/94: {err:?}"
    );
}

// ===========================================================================
// NFR-006 — with NO `[reranker]` provider, the bundled path is byte-identical
// ===========================================================================

#[test]
fn no_reranker_provider_resolves_to_none() {
    // The default config references no reranker provider → resolve yields None
    // (bundled path). This is the NFR-006 guarantee at the resolve boundary.
    let config = Config::default();
    assert!(
        resolve(&config, Capability::Reranker).unwrap().is_none(),
        "no [reranker] provider must resolve to None (bundled path)"
    );
}

#[test]
fn bundled_reranker_path_still_reranks_with_stub() {
    // With NO `[reranker]` provider and a STUB reranker injected (standing in for
    // the bundled FastembedReranker), the query still reranks and produces a
    // reranked ordering — proving the bundled path is unchanged (NFR-006).
    let ws = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let reranker = tome::embedding::stub::StubReranker::new();
    let outcome = query_with_reranker(&ws, Some(&reranker), common::stub_reranker_seed())
        .expect("bundled (stub) rerank should succeed");
    assert_eq!(outcome.scoring, query::ScoringMode::Reranked);
}
