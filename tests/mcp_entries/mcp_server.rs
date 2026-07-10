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
use tome::plugin::identity::EntryKind;
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
fn router_advertises_exactly_the_expected_tools() {
    // #497: `get_skill_info` folded into `get_skill` (behind `metadata_only`);
    // `list_plugins` / `list_catalogs` / `status` joined the read-only surface.
    // The ToolRouter's `list_all()` returns tools in registration order; the
    // assertion sorts both sides to keep the check insensitive to that detail.
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
            "list_catalogs".to_string(),
            "list_plugins".to_string(),
            "meta".to_string(),
            "search_skills".to_string(),
            "status".to_string(),
        ],
        "expected exactly the six contract-required tools (#497), got {:?}",
        names,
    );
}

#[test]
fn instructions_describe_the_discovery_flow() {
    // #497: the server instructions must name the canonical flow
    // (search_skills → get_skill metadata_only → get_skill full body), and the
    // inventory-browse + status surfaces the consolidation added.
    let env = ToolEnv::new();
    let state = build_state(&env);
    let info = Server::new(state).get_info();
    let instructions = info.instructions.expect("server advertises instructions");

    for tool in [
        "search_skills",
        "get_skill",
        "metadata_only",
        "list_plugins",
        "list_catalogs",
        "status",
    ] {
        assert!(
            instructions.contains(tool),
            "instructions must name the flow + browse/status surfaces; \
             missing `{tool}` in:\n{instructions}",
        );
    }
    // The middle tier must be framed as avoiding the full body (its whole point).
    assert!(
        instructions.contains("without loading the full body"),
        "instructions must explain metadata_only avoids the full body; got:\n{instructions}",
    );
    // #497: `get_skill_info` is gone — it must not be named.
    assert!(
        !instructions.contains("get_skill_info"),
        "instructions must not reference the removed get_skill_info tool; got:\n{instructions}",
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
        get_desc.contains("frontmatter stripped"),
        "get_skill description must reference frontmatter stripping; got: {get_desc}",
    );
    // #497: the consolidated description must name the metadata-only middle
    // tier + when_to_use guidance (formerly the get_skill_info description).
    assert!(
        get_desc.contains("metadata_only"),
        "get_skill description must reference the metadata_only mode; got: {get_desc}",
    );
    assert!(
        get_desc.contains("when_to_use"),
        "get_skill description must reference when_to_use guidance; got: {get_desc}",
    );

    // #497: `get_skill_info` is no longer advertised.
    assert!(
        !tools.iter().any(|t| t.name == "get_skill_info"),
        "get_skill_info tool must be removed from the surface",
    );

    // The three new read-only tools are advertised with behaviour-only wording.
    for (name, needle) in [
        ("list_plugins", "enabled plugins"),
        ("list_catalogs", "catalogs enrolled"),
        ("status", "Snapshot of this Tome environment"),
    ] {
        let t = tools
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("{name} advertised"));
        let desc = t.description.as_deref().unwrap_or("");
        assert!(
            desc.contains(needle),
            "{name} description must reference `{needle}`; got: {desc}",
        );
    }
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
fn get_skill_input_schema_advertises_include_resource_bodies_boolean() {
    // #333: the `get_skill` input schema (what `tools/list` exposes to an agent)
    // must advertise the optional `include_resource_bodies` boolean so a client
    // can discover the inline-resource mode. It carries `#[serde(default)]` ⇒ it
    // is NOT in `required`. A rename, type change, or accidental promotion to
    // required flips this test red.
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

    let flag = properties
        .get("include_resource_bodies")
        .expect("input schema advertises the `include_resource_bodies` property (#333)");
    assert_eq!(
        flag.get("type").and_then(|t| t.as_str()),
        Some("boolean"),
        "`include_resource_bodies` must be a boolean in the input schema; got: {flag}",
    );

    let required: Vec<&str> = schema
        .get("required")
        .and_then(|r| r.as_array())
        .expect("get_skill input schema has a `required` array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        !required.contains(&"include_resource_bodies"),
        "`include_resource_bodies` is defaulted (optional) and must not be required; got: {required:?}",
    );
}

#[test]
fn get_skill_output_schema_advertises_resource_bodies_array() {
    // #333: the `get_skill` OUTPUT schema (derived by the #[tool] macro from
    // `Result<Json<Output>, _>`) must advertise the optional `resource_bodies`
    // property as an array of `{ path, content }` objects, so a client can
    // discover the inlined-resource view. It is `Option` (skip_serializing_if)
    // ⇒ NOT required.
    let tools = Server::tool_router().list_all();
    let get = tools
        .iter()
        .find(|t| t.name == "get_skill")
        .expect("get_skill advertised");

    let output_schema = get
        .output_schema
        .as_ref()
        .expect("get_skill advertises an output schema");
    let properties = output_schema
        .get("properties")
        .and_then(|p| p.as_object())
        .expect("output schema has a `properties` object");

    let rb = properties
        .get("resource_bodies")
        .expect("output schema advertises the `resource_bodies` property (#333)");

    // `resource_bodies` is `Option<Vec<ResourceBody>>`; schemars models the
    // Option as a nullable/`anyOf`-wrapped array whose `items` is a `$ref` into
    // the top-level `$defs`. Rather than pin the exact Option-wrapping shape
    // (schemars can render it as `anyOf` or `type:["array","null"]`), assert an
    // `array` shape is reachable, then resolve the element `$ref` (if any) and
    // confirm it carries `path` + `content` properties.
    let items = find_array_items(rb)
        .unwrap_or_else(|| panic!("resource_bodies must model an array; got: {rb}"));
    let item_schema = resolve_ref(output_schema.as_ref(), items);
    let item_props = item_schema
        .get("properties")
        .and_then(|p| p.as_object())
        .unwrap_or_else(|| panic!("resource_bodies items must be objects; got: {item_schema}"));
    assert!(
        item_props.contains_key("path"),
        "resource_bodies item must have a `path` field; got: {item_schema}",
    );
    assert!(
        item_props.contains_key("content"),
        "resource_bodies item must have a `content` field; got: {item_schema}",
    );

    // `resource_bodies` must NOT be in `required` (it is an Option).
    if let Some(required) = output_schema.get("required").and_then(|r| r.as_array()) {
        let required: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(
            !required.contains(&"resource_bodies"),
            "`resource_bodies` is optional and must not be required; got: {required:?}",
        );
    }
}

/// Locate the array-`items` object for a schemars-generated property that may
/// be a bare array (`{"type":"array","items":{…}}`) OR an Option-wrapped array
/// (rendered as `anyOf`/`oneOf` with one array branch, or
/// `type:["array","null"]`). Returns the `items` object of the array branch.
fn find_array_items(schema: &serde_json::Value) -> Option<&serde_json::Value> {
    fn is_array_type(v: &serde_json::Value) -> bool {
        match v.get("type") {
            Some(serde_json::Value::String(s)) => s == "array",
            Some(serde_json::Value::Array(types)) => {
                types.iter().any(|t| t.as_str() == Some("array"))
            }
            _ => false,
        }
    }
    if is_array_type(schema) {
        return schema.get("items");
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(branches) = schema.get(key).and_then(|b| b.as_array()) {
            for branch in branches {
                if is_array_type(branch) {
                    return branch.get("items");
                }
            }
        }
    }
    None
}

/// Resolve a schemars `{"$ref":"#/$defs/Name"}` node against the top-level
/// schema's `$defs`; a non-`$ref` node is returned as-is. Only the local
/// `#/$defs/<Name>` form (what schemars emits) is followed.
fn resolve_ref<'a>(
    root: &'a serde_json::Map<String, serde_json::Value>,
    node: &'a serde_json::Value,
) -> &'a serde_json::Value {
    let Some(reference) = node.get("$ref").and_then(|r| r.as_str()) else {
        return node;
    };
    let name = reference
        .strip_prefix("#/$defs/")
        .unwrap_or_else(|| panic!("unexpected $ref form: {reference}"));
    root.get("$defs")
        .and_then(|d| d.get(name))
        .unwrap_or_else(|| panic!("$ref `{reference}` not found in $defs"))
}

#[test]
fn get_skill_kind_schema_description_names_all_valid_kinds() {
    // #332/#497: the consolidated `get_skill` input schema's `kind` property
    // description (what `tools/list` exposes to an agent) must enumerate the
    // valid values so a caller knows `command` / `agent` are selectable, not
    // only the defaulted `skill`.
    let tools = Server::tool_router().list_all();
    let get = tools
        .iter()
        .find(|t| t.name == "get_skill")
        .expect("get_skill advertised");

    let properties = get
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
fn get_skill_input_schema_advertises_metadata_only_boolean() {
    // #497: the consolidated `get_skill` input schema must advertise the
    // optional `metadata_only` boolean so a client can discover the middle
    // tier. It carries `#[serde(default)]` ⇒ NOT in `required`.
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
    let flag = properties
        .get("metadata_only")
        .expect("input schema advertises the `metadata_only` property (#497)");
    assert_eq!(
        flag.get("type").and_then(|t| t.as_str()),
        Some("boolean"),
        "`metadata_only` must be a boolean; got: {flag}",
    );

    let required: Vec<&str> = schema
        .get("required")
        .and_then(|r| r.as_array())
        .expect("get_skill input schema has a `required` array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        !required.contains(&"metadata_only"),
        "`metadata_only` is defaulted (optional) and must not be required; got: {required:?}",
    );
}

#[test]
fn search_skills_input_schema_advertises_kind_min_score_and_rerank() {
    // #320 / #502: the `search_skills` input schema (what `tools/list` exposes to
    // an agent) must advertise the OPTIONAL knobs so a client can discover them:
    // `kind` (a closed enum skill/command/agent), `min_score` (a number), and
    // `rerank` (a boolean — the #502 per-call reranking override). All carry
    // `#[serde(default)]` ⇒ NONE is in `required`. A rename, type change, dropped
    // enum variant, or accidental promotion to required flips this test red.
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

    // -- `rerank`: an optional boolean (#502) ----------------------------------
    // schemars renders `Option<bool>` as `type: ["boolean", "null"]`. This is
    // the per-call override that lets an agent turn reranking on/off now that
    // reranking is off by default.
    let rerank = properties
        .get("rerank")
        .expect("input schema advertises the `rerank` property (#502)");
    let rerank_types: Vec<&str> = rerank
        .get("type")
        .and_then(|t| t.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .expect("`rerank` carries a `type` array");
    assert!(
        rerank_types.contains(&"boolean"),
        "`rerank` must be a boolean in the input schema; got type: {rerank_types:?}",
    );

    // -- `required`: only `query`; the defaulted filters must NOT appear -------
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
    for defaulted in ["kind", "min_score", "rerank"] {
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
                rerank: None,
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
                rerank: None,
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
                rerank: None,
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
                rerank: None,
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
                kind: EntryKind::Skill,
                metadata_only: false,
                raw: false,
                include_resource_bodies: false,
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
                kind: EntryKind::Skill,
                metadata_only: false,
                raw: false,
                include_resource_bodies: false,
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
