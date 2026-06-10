//! Phase 5 / US1.c — byte-stable JSON wire-shape pin for
//! `prompts/get` per research §R-19.
//!
//! Pins the [`rmcp::model::GetPromptResult`] JSON serialisation so a
//! change to the rmcp `GetPromptResult` / `PromptMessage` shape or to
//! our `handle_get` wrapping logic that affects the wire output breaks
//! this test loudly.
//!
//! The F3 substitution stub returns the body unchanged, so the
//! rendered text is the same as the frontmatter-stripped body. US2+US3
//! will rewrite the body but the WRAPPER shape (description,
//! messages[0].role = "user", messages[0].content.type = "text") stays
//! pinned by this test.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};
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

use crate::common::{
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
    std::fs::write(
        plugin_dir.join("tome-plugin.toml"),
        format!(
            "name = \"{}\"\nversion = \"1.0.0\"\n",
            plugin_dir.file_name().unwrap().to_string_lossy()
        ),
    )
    .unwrap();
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

/// Same fixture discipline as `tests/mcp_prompts.rs::stage_workspace_with`
/// inlined here so the test stays self-contained.
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
            "---\nname: hello\ndescription: Greet someone politely.\n---\nHello, world!",
        )],
    );

    let embedder = StubEmbedder::new();
    let scope = Scope(WorkspaceName::global());
    let deps = build_deps(&paths, &config, &embedder, &scope);
    let id: PluginId = "acme/p".parse().unwrap();

    // Seed central DB catalog enrolment + symlink the URL-hashed cache
    // dir to the on-disk fixture so the registry's resolve walk works.
    // FF1: this must precede `lifecycle::enable`, which now resolves the
    // plugin dir from `workspace_catalogs`.
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
        host_harness: None,
    })
}

#[test]
fn prompts_get_payload_is_byte_stable_for_single_user_message_with_description() {
    let (_tmp, paths) = stage_one_command_workspace();
    let state = build_state(&paths);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let response = rt
        .block_on(prompts::handle_get(state, "p__hello".into(), None))
        .expect("prompts/get ok");

    let serialised = serde_json::to_value(&response).expect("serialise");

    // rmcp's `GetPromptResult` shape: `description: Option<String>`
    // skipped when None; `messages: Vec<PromptMessage>` always present.
    // `PromptMessage.role` is camelCase-serialised (enum variant
    // `User` → `"user"`); `content` is internally tagged on `type`.
    // The F3 stub leaves the body unchanged, so the rendered text is
    // the frontmatter-stripped body verbatim.
    let expected: Value = json!({
        "description": "Greet someone politely.",
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": "Hello, world!"
                }
            }
        ]
    });

    assert_eq!(
        serialised,
        expected,
        "prompts/get wire-shape drift detected;\n  got:      {}\n  expected: {}",
        serde_json::to_string_pretty(&serialised).unwrap(),
        serde_json::to_string_pretty(&expected).unwrap(),
    );
}
