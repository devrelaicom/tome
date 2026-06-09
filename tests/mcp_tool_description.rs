//! Phase 4 / US4.b — T335: FR-425 MCP `search_skills` tool
//! description composition.
//!
//! Asserts:
//!
//! * When the workspace's `settings.toml` has a non-empty
//!   `[summaries].short`, the runtime tool description includes it.
//! * When the cached summary is absent, the description falls back
//!   to the fixed scaffold without panicking.
//! * An oversized cached summary emits a `tracing::warn!` but the
//!   composition still returns the (oversized) string — the server
//!   does NOT refuse to start.
//!
//! Tests inspect [`tome::mcp::tool_description::compose`] directly
//! and use `Server::override_search_skills_description` +
//! `tool_router_ref` to confirm the runtime override path mutates
//! the registered tool's `attr.description` (the same surface rmcp
//! advertises in `list_tools`).

mod common;

use common::lifecycle_paths;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::OnceCell;
use tome::embedding::registry::{MODEL_REGISTRY, ModelEntry, ModelKind};
use tome::embedding::stub::{StubEmbedder, StubReranker};
use tome::embedding::{Embedder, Reranker};
use tome::mcp::prompts::PromptRegistry;
use tome::mcp::server::Server;
use tome::mcp::state::McpState;
use tome::mcp::tool_description::{MAX_DESCRIPTION_LEN, SCAFFOLD, compose, warn_if_too_long};
use tome::paths::Paths;
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

fn entry_for(kind: ModelKind) -> &'static ModelEntry {
    MODEL_REGISTRY
        .iter()
        .find(|e| e.kind == kind)
        .expect("registry has entry")
}

fn build_state(paths: &Paths, ws: &WorkspaceName) -> Arc<McpState> {
    let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new());
    let _reranker: Arc<dyn Reranker> = Arc::new(StubReranker::new());
    let scope = ResolvedScope {
        scope: Scope(ws.clone()),
        source: ScopeSource::GlobalFallback,
        project_root: None,
    };
    Arc::new(McpState {
        embedder,
        reranker: OnceCell::new(),
        scope,
        paths: paths.clone(),
        embedder_entry: entry_for(ModelKind::Embedder),
        reranker_entry: entry_for(ModelKind::Reranker),
        prompt_registry: Arc::new(PromptRegistry::default()),
        host_harness: None,
    })
}

#[test]
fn description_falls_back_to_scaffold_when_settings_absent() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    let ws = WorkspaceName::parse("demo").unwrap();
    let composed = compose(&ws, &paths);
    assert_eq!(composed, SCAFFOLD);
}

#[test]
fn description_includes_cached_short_when_present() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    let ws = WorkspaceName::parse("demo").unwrap();
    std::fs::create_dir_all(paths.workspace_dir(&ws)).unwrap();
    std::fs::write(
        paths.workspace_settings_file(&ws),
        "name = \"demo\"\n[summaries]\nshort = \"focuses on testing patterns and CLI plumbing\"\nlong = \"long body\"\n",
    )
    .unwrap();

    let composed = compose(&ws, &paths);
    assert!(composed.starts_with(SCAFFOLD));
    assert!(composed.contains("focuses on testing patterns and CLI plumbing"));
}

#[test]
fn oversized_description_still_returned_and_server_can_apply_it() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    let ws = WorkspaceName::parse("demo").unwrap();
    std::fs::create_dir_all(paths.workspace_dir(&ws)).unwrap();

    // Build a 5000-char `short` summary — well above the
    // MAX_DESCRIPTION_LEN soft cap.
    let oversized = "x".repeat(5_000);
    let body = format!("name = \"demo\"\n[summaries]\nshort = \"{oversized}\"\nlong = \"long\"\n");
    std::fs::write(paths.workspace_settings_file(&ws), body).unwrap();

    let composed = compose(&ws, &paths);
    assert!(
        composed.chars().count() > MAX_DESCRIPTION_LEN,
        "test setup: composed must exceed the soft cap",
    );

    // warn_if_too_long is best-effort — should NOT panic and should
    // not mutate the description. We can't easily intercept the
    // tracing event from a test, but we can confirm the call returns
    // cleanly.
    warn_if_too_long(&ws, &composed);

    // The server's override path must accept the oversized string
    // without refusing to start. Verify by mutating the router and
    // reading the tool's advertised description back.
    let state = build_state(&paths, &ws);
    let mut server = Server::new(state);
    server.override_search_skills_description(composed.clone());

    let tools = server.tool_router_ref().list_all();
    let search = tools
        .iter()
        .find(|t| t.name == "search_skills")
        .expect("search_skills tool registered");
    let desc = search.description.as_deref().unwrap_or("");
    assert_eq!(
        desc.chars().count(),
        composed.chars().count(),
        "override should land verbatim regardless of length",
    );
}

#[test]
fn server_override_path_mutates_advertised_description() {
    // End-to-end: override the description through the same seam
    // `mcp::run` uses (`server::override_search_skills_description`)
    // and confirm `tool_router.list_all()` reflects the new value.
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    let ws = WorkspaceName::parse("demo").unwrap();
    std::fs::create_dir_all(paths.workspace_dir(&ws)).unwrap();
    std::fs::write(
        paths.workspace_settings_file(&ws),
        "name = \"demo\"\n[summaries]\nshort = \"specialised in payment integrations\"\nlong = \"l\"\n",
    )
    .unwrap();

    let state = build_state(&paths, &ws);
    let mut server = Server::new(state);
    let composed = compose(&ws, &paths);
    server.override_search_skills_description(composed.clone());

    let tools = server.tool_router_ref().list_all();
    let search = tools
        .iter()
        .find(|t| t.name == "search_skills")
        .expect("search_skills tool registered");
    let desc = search.description.as_deref().unwrap_or("");
    assert!(
        desc.contains("specialised in payment integrations"),
        "advertised description should include the cached short summary, got:\n{desc}",
    );
    assert!(
        desc.starts_with(SCAFFOLD),
        "advertised description should retain the scaffold prefix",
    );
}
