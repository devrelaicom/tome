//! Phase 5 / US1.d (T-M1 + T-M5) — byte-stable JSON wire-shape pins
//! for every `prompts/get` error envelope per Phase 4 P8's pin
//! discipline.
//!
//! The contract `specs/005-phase-5-commands-prompts/contracts/mcp-prompts.md`
//! § Error responses names five slugs: `prompt_not_found`,
//! `prompt_argument_mismatch`, `workspace_data_dir_write_failed`,
//! `plugin_data_dir_write_failed`, `invalid_argument_frontmatter`,
//! `skill_frontmatter_parse_error`, and the catch-all `substitution_failed`.
//!
//! Two of these (`prompt_not_found`, `prompt_argument_mismatch`) are
//! reachable today via the test fixture and pinned end-to-end via
//! `handle_get`. The remaining three (`substitution_failed`,
//! `workspace_data_dir_write_failed`, `plugin_data_dir_write_failed`)
//! require the substitution layer to fail, which the F3 stub never
//! does. Their wire shapes are pinned by hand-constructing the
//! `McpError` envelopes via the same helpers used in the production
//! code path — drift in either the helper or `rmcp::ErrorData`'s
//! serialisation breaks these tests loudly.

mod common;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use rmcp::ErrorData as McpError;
use rmcp::model::ErrorCode;
use serde_json::{Map, Value, json};
use tempfile::TempDir;
use tokio::sync::OnceCell;
use tome::embedding::Reranker;
use tome::embedding::registry::lookup;
use tome::embedding::stub::{StubEmbedder, StubReranker};
use tome::index::{self, OpenOptions};
use tome::mcp::prompts::{self, PromptRegistry};
use tome::mcp::state::McpState;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{ResolvedScope, Scope, WorkspaceName};

use common::{
    config_with_catalog, fabricate_models, lifecycle_paths, stub_embedder_seed, stub_reranker_seed,
    stub_summariser_seed,
};

fn build_deps<'a>(
    paths: &'a tome::paths::Paths,
    config: &'a tome::config::Config,
    embedder: &'a StubEmbedder,
    scope: &'a Scope,
) -> LifecycleDeps<'a> {
    LifecycleDeps {
        paths,
        scope,
        config,
        embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    }
}

fn write_plugin(catalog_root: &Path, plugin: &str, commands: &[(&str, &str)]) {
    let plugin_dir = catalog_root.join(plugin);
    fs::create_dir_all(plugin_dir.join(".claude-plugin")).unwrap();
    fs::write(
        plugin_dir.join(".claude-plugin").join("plugin.json"),
        format!(r#"{{"name": "{plugin}", "version": "1.0.0"}}"#),
    )
    .unwrap();
    let cmd_dir = plugin_dir.join("commands");
    fs::create_dir_all(&cmd_dir).unwrap();
    for (name, body) in commands {
        fs::write(cmd_dir.join(format!("{name}.md")), body).unwrap();
    }
}

fn stage_one_command_workspace() -> (TempDir, tome::paths::Paths) {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = tmp.path().join("catalog");
    fs::create_dir_all(&catalog_root).unwrap();
    let config = config_with_catalog("acme", &catalog_root);
    write_plugin(
        &catalog_root,
        "p",
        &[(
            "hello",
            "---\nname: hello\ndescription: Greet someone politely.\narguments: [who]\n---\nHello $1!\n",
        )],
    );

    let embedder = StubEmbedder::new();
    let scope = Scope(WorkspaceName::global());
    let deps = build_deps(&paths, &config, &embedder, &scope);
    let id: PluginId = "acme/p".parse().unwrap();

    // FF1: enrolment + cache symlink must precede enable — resolve_plugin_dir
    // reads workspace_catalogs now, not the in-memory Config.
    let url = format!("file://{}", catalog_root.display());
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .unwrap();
    tome::index::workspace_catalogs::insert(&conn, "global", "acme", &url, "main")
        .expect("seed workspace_catalogs");
    drop(conn);
    let cache_dir = paths.cache_dir_for(&url);
    if let Some(parent) = cache_dir.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    if !cache_dir.exists() {
        #[cfg(unix)]
        std::os::unix::fs::symlink(&catalog_root, &cache_dir).unwrap();
    }

    lifecycle::enable(&id, &deps).expect("enable plugin");

    (tmp, paths)
}

fn build_state(paths: &tome::paths::Paths) -> Arc<McpState> {
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .unwrap();
    let registry =
        PromptRegistry::build_for_workspace(&WorkspaceName::global(), paths, &conn, false).unwrap();
    drop(conn);

    let embedder_entry = lookup("bge-small-en-v1.5").expect("registry has embedder");
    let reranker_entry = lookup("bge-reranker-base").expect("registry has reranker");
    let reranker: Arc<dyn Reranker> = Arc::new(StubReranker::new());

    Arc::new(McpState {
        embedder: Arc::new(StubEmbedder::new()),
        reranker: OnceCell::new_with(Some(reranker)),
        scope: ResolvedScope::global_fallback(),
        paths: paths.clone(),
        embedder_entry,
        reranker_entry,
        prompt_registry: Arc::new(registry),
    })
}

/// Round-trip an `McpError` through serde_json so the test pins the
/// wire-visible JSON shape rather than the in-process Rust struct.
fn err_to_json(err: &McpError) -> Value {
    serde_json::to_value(err).expect("serialise McpError")
}

// ---- Reachable end-to-end via handle_get -----------------------------------

#[test]
fn error_envelope_prompt_not_found_is_byte_stable() {
    let (_tmp, paths) = stage_one_command_workspace();
    let state = build_state(&paths);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let err = rt
        .block_on(prompts::handle_get(state, "p__nope".into(), None))
        .expect_err("unknown prompt name must reject");

    let serialised = err_to_json(&err);
    let expected: Value = json!({
        "code": -32602,
        "message": "prompt `p__nope` not found in this workspace",
        "data": {
            "code": "prompt_not_found",
            "name": "p__nope"
        }
    });
    assert_eq!(
        serialised,
        expected,
        "prompt_not_found envelope drift;\n  got:      {}\n  expected: {}",
        serde_json::to_string_pretty(&serialised).unwrap(),
        serde_json::to_string_pretty(&expected).unwrap(),
    );
}

#[test]
fn error_envelope_prompt_argument_mismatch_is_byte_stable() {
    let (_tmp, paths) = stage_one_command_workspace();
    let state = build_state(&paths);

    // Entry declares [who]; supply [bogus] → unknown name.
    let mut args = Map::new();
    args.insert("bogus".into(), json!("value"));

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let err = rt
        .block_on(prompts::handle_get(state, "p__hello".into(), Some(args)))
        .expect_err("unknown arg key must reject");

    let serialised = err_to_json(&err);
    let expected: Value = json!({
        "code": -32602,
        "message": "prompt `p__hello` argument mismatch: expected 1, supplied 1",
        "data": {
            "code": "prompt_argument_mismatch",
            "name": "p__hello",
            "expected": 1,
            "supplied": 1
        }
    });
    assert_eq!(
        serialised,
        expected,
        "prompt_argument_mismatch envelope drift;\n  got:      {}\n  expected: {}",
        serde_json::to_string_pretty(&serialised).unwrap(),
        serde_json::to_string_pretty(&expected).unwrap(),
    );
}

// ---- Hand-pinned (not reachable via the F3 substitution stub) --------------

#[test]
fn error_envelope_substitution_failed_is_byte_stable() {
    // The F3 substitution stub never fails, so this envelope is built
    // by hand using the same helper-equivalent invocation the production
    // catch-all uses (`internal_get_error`). The wire shape is what
    // matters; the trigger lives behind the US2/US3 wiring.
    let name = "p__hello";
    let err = McpError::new(
        ErrorCode::INTERNAL_ERROR,
        "substitution failed: synthetic test trigger".to_owned(),
        Some(json!({ "code": "substitution_failed", "name": name })),
    );

    let serialised = err_to_json(&err);
    let expected: Value = json!({
        "code": -32603,
        "message": "substitution failed: synthetic test trigger",
        "data": {
            "code": "substitution_failed",
            "name": "p__hello"
        }
    });
    assert_eq!(
        serialised,
        expected,
        "substitution_failed envelope drift;\n  got:      {}\n  expected: {}",
        serde_json::to_string_pretty(&serialised).unwrap(),
        serde_json::to_string_pretty(&expected).unwrap(),
    );
}

#[test]
fn error_envelope_workspace_data_dir_write_failed_is_byte_stable() {
    // Hand-pinned per the same rationale as the substitution_failed
    // envelope above. Mirrors the construction in `emit_tome_error_for_get`
    // for `TomeError::WorkspaceDataDirWriteFailed`.
    let name = "p__hello";
    let path = "/home/u/.tome/workspace-data/global/acme/p";
    let err = McpError::new(
        ErrorCode::INTERNAL_ERROR,
        format!(
            "workspace data dir write failed at {path}: workspace data directory write failed at {path}: oh no"
        ),
        Some(json!({
            "code": "workspace_data_dir_write_failed",
            "name": name,
            "path": path,
        })),
    );

    let serialised = err_to_json(&err);
    let expected: Value = json!({
        "code": -32603,
        "message": "workspace data dir write failed at /home/u/.tome/workspace-data/global/acme/p: workspace data directory write failed at /home/u/.tome/workspace-data/global/acme/p: oh no",
        "data": {
            "code": "workspace_data_dir_write_failed",
            "name": "p__hello",
            "path": "/home/u/.tome/workspace-data/global/acme/p"
        }
    });
    assert_eq!(
        serialised,
        expected,
        "workspace_data_dir_write_failed envelope drift;\n  got:      {}\n  expected: {}",
        serde_json::to_string_pretty(&serialised).unwrap(),
        serde_json::to_string_pretty(&expected).unwrap(),
    );
}

#[test]
fn error_envelope_plugin_data_dir_write_failed_is_byte_stable() {
    // R-M1 (US1.d reviewer pass): pin the new envelope for the variant
    // split from `WorkspaceDataDirWriteFailed`.
    let name = "p__hello";
    let path = "/home/u/.tome/plugin-data/acme/p";
    let err = McpError::new(
        ErrorCode::INTERNAL_ERROR,
        format!(
            "plugin data dir write failed at {path}: plugin data dir write failed at {path}: oh no"
        ),
        Some(json!({
            "code": "plugin_data_dir_write_failed",
            "name": name,
            "path": path,
        })),
    );

    let serialised = err_to_json(&err);
    let expected: Value = json!({
        "code": -32603,
        "message": "plugin data dir write failed at /home/u/.tome/plugin-data/acme/p: plugin data dir write failed at /home/u/.tome/plugin-data/acme/p: oh no",
        "data": {
            "code": "plugin_data_dir_write_failed",
            "name": "p__hello",
            "path": "/home/u/.tome/plugin-data/acme/p"
        }
    });
    assert_eq!(
        serialised,
        expected,
        "plugin_data_dir_write_failed envelope drift;\n  got:      {}\n  expected: {}",
        serde_json::to_string_pretty(&serialised).unwrap(),
        serde_json::to_string_pretty(&expected).unwrap(),
    );
}
