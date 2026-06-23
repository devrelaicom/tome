//! `tome mcp` server-level behaviour. Covers the parts of T092 that are
//! tractable without spawning a real `rmcp` handshake or loading the
//! 345 MB BGE ONNX models:
//!
//! - The `ToolRouter` advertises exactly the three tools the contract names
//!   (`search_skills`, `get_skill_info`, `get_skill`). Phase 5 / US4.a
//!   widened the two-tool surface to three by adding the middle-tier
//!   `get_skill_info` between discovery and full-body fetch.
//! - The advertised descriptions match the contract's normative wording
//!   and do NOT enumerate any specific catalog / plugin / skill name
//!   that lives in the test fixture (FR-108 — "the description must not
//!   enumerate any specific catalog, plugin, or skill name").
//! - The handlers' input-validation paths produce the contract's
//!   structured error codes (`plugin_without_catalog`, `unknown_catalog`,
//!   bounds checks).
//!
//! Deferred to manual SC-001 / SC-002 verification (see
//! `retro/P3.md` § "T088 manual verification"):
//! - End-to-end happy paths for both tools against a real BGE-indexed
//!   fixture catalog.
//! - The full MCP handshake captured on stdout, asserted byte-by-byte
//!   to be protocol traffic only (T093 protocol purity).
//! - Release-mode latency budget (T094: p50 < 300 ms, p99 < 600 ms).
//! - SIGINT graceful shutdown with in-flight tool calls (T095).

use std::sync::Arc;

use crate::common::mcp_harness::open_index;
use crate::common::{ToolEnv, paths_for};
use tokio::sync::OnceCell;
use tome::embedding::Reranker;
use tome::embedding::registry::{ModelKind, lookup};
use tome::embedding::stub::{StubEmbedder, StubReranker};
use tome::mcp::prompts::PromptRegistry;
use tome::mcp::server::Server;
use tome::mcp::state::McpState;
use tome::mcp::tools::{get_skill, search_skills};
use tome::workspace::ResolvedScope;

/// Build a minimal `McpState` rooted in an isolated `ToolEnv`. The
/// caller decides whether to pre-write the config file; this builder
/// only sets up the path-resolution side.
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
        prompt_registry: Arc::new(std::sync::RwLock::new(Arc::new(PromptRegistry::default()))),
        host_harness: None,
        last_search_ranks: std::sync::Mutex::new(std::collections::HashMap::new()),
        flush_signal: std::sync::Arc::new(tokio::sync::Notify::new()),
        enqueued_since_flush: std::sync::atomic::AtomicUsize::new(0),
    })
}

#[test]
fn router_advertises_exactly_four_tools() {
    // Phase 5 / US4.a: `get_skill_info` joins `search_skills` + `get_skill`.
    // Phase 9 / US3: the built-in `meta` tool joins them. The ToolRouter's
    // `list_all()` returns tools in registration order; the assertion sorts both
    // sides to keep the check insensitive to that detail.
    let mut names: Vec<String> = Server::tool_router()
        .list_all()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "get_skill".to_string(),
            "get_skill_info".to_string(),
            "meta".to_string(),
            "search_skills".to_string(),
        ],
        "expected exactly the four contract-required tools, got {:?}",
        names,
    );
}

#[test]
fn descriptions_match_contract_wording() {
    let tools = Server::tool_router().list_all();

    let search = tools
        .iter()
        .find(|t| t.name == "search_skills")
        .expect("search_skills advertised");
    let search_desc = search.description.as_deref().unwrap_or("");
    assert!(
        search_desc.contains("most relevant skills"),
        "search_skills description must reference 'most relevant skills'; got: {search_desc}",
    );
    assert!(
        search_desc.contains("get_skill"),
        "search_skills description must reference the follow-up `get_skill` tool; got: {search_desc}",
    );

    let get = tools
        .iter()
        .find(|t| t.name == "get_skill")
        .expect("get_skill advertised");
    let get_desc = get.description.as_deref().unwrap_or("");
    assert!(
        get_desc.contains("body of one skill"),
        "get_skill description must reference 'body of one skill'; got: {get_desc}",
    );
    assert!(
        get_desc.contains("frontmatter stripped"),
        "get_skill description must reference frontmatter stripping; got: {get_desc}",
    );

    let info = tools
        .iter()
        .find(|t| t.name == "get_skill_info")
        .expect("get_skill_info advertised");
    let info_desc = info.description.as_deref().unwrap_or("");
    assert!(
        info_desc.contains("without loading its full body"),
        "get_skill_info description must explain it's the middle-tier (avoids full body); got: {info_desc}",
    );
    assert!(
        info_desc.contains("when_to_use"),
        "get_skill_info description must reference when_to_use guidance; got: {info_desc}",
    );
}

#[test]
fn descriptions_do_not_enumerate_fixture_identifiers() {
    // FR-108: the tool descriptions must NOT name any specific catalog,
    // plugin, or skill identifier. We assert against the identifiers
    // present in `tests/fixtures/sample-plugin-catalog/` so that if a
    // future edit leaks one of them into the wording the test catches
    // it. The list is conservative — anything that looks like a name
    // from the fixture should appear here.
    let leakage_substrings = [
        // Catalog/plugin identifiers from sample-plugin-catalog.
        "sample-plugin-catalog",
        "writers",
        "blog-post-skeleton",
        "midnight-experts",
        "compact-expert",
    ];

    let descriptions: Vec<String> = Server::tool_router()
        .list_all()
        .into_iter()
        .map(|t| t.description.as_deref().unwrap_or("").to_string())
        .collect();

    for desc in &descriptions {
        for needle in leakage_substrings {
            assert!(
                !desc.contains(needle),
                "FR-108 violation: tool description leaks identifier `{needle}`. \
                 Descriptions must reference behaviour, not specific catalog / \
                 plugin / skill names. Offending description:\n{desc}",
            );
        }
    }
}

#[test]
fn search_skills_rejects_top_k_out_of_range() {
    let env = ToolEnv::new();
    let state = build_state(&env);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    // top_k = 0 → invalid_params
    let err = rt
        .block_on(search_skills::handle(
            state.clone(),
            search_skills::Input {
                query: "hello".into(),
                top_k: Some(0),
                catalog: None,
                plugin: None,
                description_max_chars: 150,
            },
        ))
        .expect_err("top_k = 0 must reject");
    assert!(
        err.message.contains("top_k"),
        "expected top_k bounds error message, got: {}",
        err.message,
    );

    // top_k = 101 → invalid_params
    let err = rt
        .block_on(search_skills::handle(
            state,
            search_skills::Input {
                query: "hello".into(),
                top_k: Some(101),
                catalog: None,
                plugin: None,
                description_max_chars: 150,
            },
        ))
        .expect_err("top_k = 101 must reject");
    assert!(
        err.message.contains("top_k"),
        "expected top_k bounds error message, got: {}",
        err.message,
    );
}

#[test]
fn search_skills_rejects_plugin_without_catalog() {
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
                query: "hello".into(),
                top_k: Some(10),
                catalog: None,
                plugin: Some("writers".into()),
                description_max_chars: 150,
            },
        ))
        .expect_err("plugin without catalog must reject");

    let data = err.data.expect("structured error data");
    assert_eq!(
        data.get("code").and_then(|c| c.as_str()),
        Some("plugin_without_catalog"),
        "expected `plugin_without_catalog` code in data, got: {data}",
    );
}

#[test]
fn search_skills_returns_unknown_catalog_for_missing_name() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // FF3: catalog existence resolves from the `workspace_catalogs` DB, not
    // config.toml. Bootstrap an empty index DB (the MCP preflight requires
    // the DB to exist before any handler runs); `find` then returns None for
    // the unenrolled catalog → `unknown_catalog`.
    drop(open_index(&paths));

    let state = build_state(&env);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    let err = rt
        .block_on(search_skills::handle(
            state,
            search_skills::Input {
                query: "hello".into(),
                top_k: Some(10),
                catalog: Some("nonexistent".into()),
                plugin: None,
                description_max_chars: 150,
            },
        ))
        .expect_err("unknown catalog must reject");

    let data = err.data.expect("structured error data");
    assert_eq!(
        data.get("code").and_then(|c| c.as_str()),
        Some("unknown_catalog"),
        "expected `unknown_catalog` code in data, got: {data}",
    );
    assert_eq!(
        data.get("catalog").and_then(|c| c.as_str()),
        Some("nonexistent"),
        "expected catalog name to round-trip in data, got: {data}",
    );
}

#[test]
fn get_skill_returns_unknown_catalog_for_missing_name() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // FF3: catalog existence resolves from the `workspace_catalogs` DB, not
    // config.toml. Bootstrap an empty index DB (the MCP preflight requires
    // the DB to exist before any handler runs); `find` then returns None for
    // the unenrolled catalog → `unknown_catalog`.
    drop(open_index(&paths));

    let state = build_state(&env);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    let err = rt
        .block_on(get_skill::handle(
            state,
            get_skill::Input {
                catalog: "nonexistent".into(),
                plugin: "p".into(),
                name: "s".into(),
            },
        ))
        .expect_err("unknown catalog must reject");

    let data = err.data.expect("structured error data");
    assert_eq!(
        data.get("code").and_then(|c| c.as_str()),
        Some("unknown_catalog"),
        "expected `unknown_catalog` code in data, got: {data}",
    );
}

#[test]
fn get_skill_rejects_empty_fields() {
    let env = ToolEnv::new();
    let state = build_state(&env);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    let err = rt
        .block_on(get_skill::handle(
            state,
            get_skill::Input {
                catalog: "".into(),
                plugin: "p".into(),
                name: "s".into(),
            },
        ))
        .expect_err("empty catalog must reject");
    assert!(
        err.message.contains("non-empty"),
        "expected empty-field rejection message, got: {}",
        err.message,
    );
}

// Silence the unused-import warning on platforms / cfg branches where
// `ModelKind` isn't directly referenced.
#[allow(dead_code)]
fn _kind_assertion(k: ModelKind) -> bool {
    matches!(k, ModelKind::Embedder | ModelKind::Reranker)
}
