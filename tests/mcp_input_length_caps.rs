//! Phase 4 / US5.a (T373) — MCP `search_skills.query` length cap test.
//!
//! Folds in the P8-deferred review item: queries strictly longer than
//! `MAX_QUERY_CHARS` (4096 chars per research §R-17) must be rejected
//! with a dedicated error envelope (`code: "query_too_long"`); queries
//! exactly at the cap MUST be accepted (boundary-tight). The handler
//! reuses Phase 3's `invalid_params` shape, so we assert on the
//! structured `data.code` field per `contracts/mcp-tools.md`.

mod common;

use std::sync::Arc;

use common::{ToolEnv, paths_for};
use tokio::sync::OnceCell;
use tome::embedding::Reranker;
use tome::embedding::registry::lookup;
use tome::embedding::stub::{StubEmbedder, StubReranker};
use tome::mcp::prompts::PromptRegistry;
use tome::mcp::state::McpState;
use tome::mcp::tools::search_skills::{self, MAX_QUERY_CHARS};
use tome::workspace::ResolvedScope;

fn build_state(env: &ToolEnv) -> Arc<McpState> {
    let paths = paths_for(env);
    let embedder_entry = lookup("bge-small-en-v1.5").expect("registry has embedder");
    let reranker_entry = lookup("bge-reranker-base").expect("registry has reranker");
    let reranker: Arc<dyn Reranker> = Arc::new(StubReranker::new());
    Arc::new(McpState {
        embedder: Arc::new(StubEmbedder::new()),
        reranker: OnceCell::new_with(Some(reranker)),
        scope: ResolvedScope::global_fallback(),
        paths,
        embedder_entry,
        reranker_entry,
        prompt_registry: Arc::new(PromptRegistry::default()),
    })
}

#[test]
fn rejects_query_strictly_longer_than_cap_with_dedicated_code() {
    let env = ToolEnv::new();
    let state = build_state(&env);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    // One character past the cap.
    let query: String = "a".repeat(MAX_QUERY_CHARS + 1);
    let err = rt
        .block_on(search_skills::handle(
            state,
            search_skills::Input {
                query,
                top_k: 10,
                catalog: None,
                plugin: None,
                description_max_chars: 150,
            },
        ))
        .expect_err("query > MAX_QUERY_CHARS must reject");
    let data = err.data.expect("structured error envelope");
    assert_eq!(
        data.get("code").and_then(|c| c.as_str()),
        Some("query_too_long"),
        "expected `query_too_long` code in data, got: {data}",
    );
    assert_eq!(
        data.get("max_chars").and_then(|n| n.as_u64()),
        Some(MAX_QUERY_CHARS as u64),
        "expected max_chars hint in data, got: {data}",
    );
}

#[test]
fn accepts_query_exactly_at_cap() {
    // A query exactly MAX_QUERY_CHARS long must NOT be rejected by the
    // length validator. We don't run the full pipeline (no real models
    // here) — the assertion is that whatever error comes back, it is
    // NOT the `query_too_long` envelope. The handler proceeds past the
    // length gate; an empty catalog config will surface a different
    // error (or succeed with empty matches).
    let env = ToolEnv::new();
    let state = build_state(&env);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    let query: String = "a".repeat(MAX_QUERY_CHARS);
    let result = rt.block_on(search_skills::handle(
        state,
        search_skills::Input {
            query,
            top_k: 10,
            catalog: None,
            plugin: None,
            description_max_chars: 150,
        },
    ));
    // The boundary case may succeed or fail for OTHER reasons (no
    // config file written; reranker is stub). The point: it must NOT
    // fail with `query_too_long`.
    if let Err(err) = result
        && let Some(data) = err.data
        && let Some(code) = data.get("code").and_then(|c| c.as_str())
    {
        assert_ne!(
            code, "query_too_long",
            "query at exactly MAX_QUERY_CHARS must NOT trigger the length cap",
        );
    }
}

#[test]
fn rejects_empty_query_with_existing_error_path() {
    // Empty-query rejection is the Phase 3 path; the length cap should
    // NOT shadow it. Sanity-check that empty stays an empty-query
    // error rather than coercing into the new envelope.
    let env = ToolEnv::new();
    let state = build_state(&env);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    let err = rt
        .block_on(search_skills::handle(
            state,
            search_skills::Input {
                query: "   ".into(),
                top_k: 10,
                catalog: None,
                plugin: None,
                description_max_chars: 150,
            },
        ))
        .expect_err("empty query must reject");
    assert!(
        err.message.contains("empty"),
        "expected empty-query message, got: {}",
        err.message,
    );
}
