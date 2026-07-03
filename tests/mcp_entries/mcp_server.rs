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
use rmcp::ServerHandler;
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
        embedder_seed: tome::index::MetaSeed {
            name: embedder_entry.name.into(),
            version: embedder_entry.version.into(),
        },
        reranker_entry,
        prompt_registry: Arc::new(std::sync::RwLock::new(Arc::new(PromptRegistry::default()))),
        host_harness: None,
        last_search_ranks: std::sync::Mutex::new(std::collections::HashMap::new()),
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
fn instructions_describe_the_three_step_flow() {
    // #295: the server instructions must name the canonical THREE-step flow
    // (search_skills → get_skill_info → get_skill), so an agent uses the
    // cheaper middle tier before paying the full-body cost — not the pre-#295
    // two-step flow that omitted get_skill_info entirely.
    let env = ToolEnv::new();
    let state = build_state(&env);
    let info = Server::new(state).get_info();
    let instructions = info.instructions.expect("server advertises instructions");

    for tool in ["search_skills", "get_skill_info", "get_skill"] {
        assert!(
            instructions.contains(tool),
            "instructions must name the middle tier + both ends of the flow; \
             missing `{tool}` in:\n{instructions}",
        );
    }
    // The middle tier must be framed as avoiding the full body (its whole point).
    assert!(
        instructions.contains("without loading the full body"),
        "instructions must explain get_skill_info avoids the full body; got:\n{instructions}",
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
fn get_skill_input_schema_advertises_raw_boolean() {
    // #331: the `get_skill` input schema (what `tools/list` exposes to an
    // agent) must advertise the optional `raw` boolean property so a client
    // can discover the no-substitution mode. `raw` carries `#[serde(default)]`
    // ⇒ it is NOT in `required`. This is the schema-side pin for the new
    // parameter: a rename, type change, or accidental promotion to required
    // flips this test red.
    let tools = Server::tool_router().list_all();
    let get = tools
        .iter()
        .find(|t| t.name == "get_skill")
        .expect("get_skill advertised");

    let schema = &get.input_schema;
    let properties = schema
        .get("properties")
        .and_then(|p| p.as_object())
        .expect("input schema has a `properties` object");

    let raw = properties
        .get("raw")
        .expect("input schema advertises the `raw` property (#331)");
    assert_eq!(
        raw.get("type").and_then(|t| t.as_str()),
        Some("boolean"),
        "`raw` must be a boolean in the input schema; got: {raw}",
    );

    // The three original fields stay required; `raw` (defaulted) must NOT be.
    // Assert the `required` array UNCONDITIONALLY: a future schemars change
    // that dropped it (silently promoting/omitting fields) must fail here,
    // not skip the check. `catalog`/`plugin`/`name` must be present and `raw`
    // absent.
    let required: Vec<&str> = schema
        .get("required")
        .and_then(|r| r.as_array())
        .expect("get_skill input schema has a `required` array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    for field in ["catalog", "plugin", "name"] {
        assert!(
            required.contains(&field),
            "`{field}` must remain a required field; got: {required:?}",
        );
    }
    assert!(
        !required.contains(&"raw"),
        "`raw` is defaulted (optional) and must not appear in `required`; got: {required:?}",
    );
}

#[test]
fn get_skill_info_kind_schema_description_names_all_valid_kinds() {
    // #332: the `get_skill_info` input schema's `kind` property description
    // (what `tools/list` exposes to an agent) must enumerate the valid values
    // so a caller knows `command` / `agent` are selectable, not only the
    // defaulted `skill`. Positive pin on the reworded description.
    let tools = Server::tool_router().list_all();
    let info = tools
        .iter()
        .find(|t| t.name == "get_skill_info")
        .expect("get_skill_info advertised");

    let properties = info
        .input_schema
        .get("properties")
        .and_then(|p| p.as_object())
        .expect("input schema has a `properties` object");
    let kind = properties
        .get("kind")
        .expect("input schema advertises the `kind` property");
    let description = kind
        .get("description")
        .and_then(|d| d.as_str())
        .expect("the `kind` property carries a description");
    for value in ["skill", "command", "agent"] {
        assert!(
            description.contains(value),
            "the `kind` description must name the `{value}` value; got: {description:?}",
        );
    }
}

#[test]
fn search_skills_input_schema_advertises_kind_and_min_score() {
    // #320: the `search_skills` input schema (what `tools/list` exposes to an
    // agent) must advertise the two new OPTIONAL filters so a client can
    // discover them: `kind` (a closed enum skill/command/agent) and `min_score`
    // (a number). Both carry `#[serde(default)]` ⇒ NEITHER is in `required`.
    // A rename, type change, dropped enum variant, or accidental promotion to
    // required flips this test red.
    let tools = Server::tool_router().list_all();
    let search = tools
        .iter()
        .find(|t| t.name == "search_skills")
        .expect("search_skills advertised");
    let schema = &search.input_schema;

    let properties = schema
        .get("properties")
        .and_then(|p| p.as_object())
        .expect("input schema has a `properties` object");

    // -- `kind`: an optional closed enum (skill/command/agent) -----------------
    // schemars renders `Option<EntryKind>` as `anyOf: [ {$ref EntryKind}, null ]`,
    // with the enum values under `$defs/EntryKind`. Resolve the ref and assert
    // the three valid kinds are all present.
    let kind = properties
        .get("kind")
        .expect("input schema advertises the `kind` property (#320)");
    let kind_ref = kind
        .get("anyOf")
        .and_then(|a| a.as_array())
        .and_then(|arms| arms.iter().find_map(|arm| arm.get("$ref")))
        .and_then(|r| r.as_str())
        .expect("`kind` is `anyOf [ $ref, null ]` (an optional EntryKind)");
    assert_eq!(
        kind_ref, "#/$defs/EntryKind",
        "`kind` must reference the shared EntryKind enum; got: {kind_ref}",
    );
    let entry_kind_enum = schema
        .get("$defs")
        .and_then(|d| d.get("EntryKind"))
        .and_then(|e| e.get("enum"))
        .and_then(|e| e.as_array())
        .expect("$defs.EntryKind carries an `enum` array");
    let variants: Vec<&str> = entry_kind_enum.iter().filter_map(|v| v.as_str()).collect();
    for value in ["skill", "command", "agent"] {
        assert!(
            variants.contains(&value),
            "the `kind` enum must offer `{value}`; got: {variants:?}",
        );
    }

    // -- `min_score`: an optional number ---------------------------------------
    // schemars renders `Option<f32>` as `type: ["number", "null"]`.
    let min_score = properties
        .get("min_score")
        .expect("input schema advertises the `min_score` property (#320)");
    let types: Vec<&str> = min_score
        .get("type")
        .and_then(|t| t.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .expect("`min_score` carries a `type` array");
    assert!(
        types.contains(&"number"),
        "`min_score` must be a number in the input schema; got type: {types:?}",
    );

    // -- `required`: only `query`; the two defaulted filters must NOT appear ---
    // Assert UNCONDITIONALLY (a future schemars change that dropped it must fail
    // here, not skip the check).
    let required: Vec<&str> = schema
        .get("required")
        .and_then(|r| r.as_array())
        .expect("search_skills input schema has a `required` array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        required.contains(&"query"),
        "`query` must remain required; got: {required:?}",
    );
    for defaulted in ["kind", "min_score"] {
        assert!(
            !required.contains(&defaulted),
            "`{defaulted}` is defaulted (optional) and must not be in `required`; got: {required:?}",
        );
    }
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
                kind: None,
                min_score: None,
                description_max_chars: Some(150),
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
                kind: None,
                min_score: None,
                description_max_chars: Some(150),
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
                kind: None,
                min_score: None,
                description_max_chars: Some(150),
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
                kind: None,
                min_score: None,
                description_max_chars: Some(150),
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
                raw: false,
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
                raw: false,
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
