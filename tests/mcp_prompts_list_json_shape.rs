//! Phase 5 / US1.b — byte-stable JSON wire-shape pin for
//! `prompts/list` per research §R-19.
//!
//! Pins the `Vec<Prompt>` JSON serialisation (the inner payload of the
//! `ListPromptsResult.prompts` field — rmcp wraps the envelope itself,
//! we own the array). A change to the rmcp `Prompt` shape or to our
//! descriptor construction logic that affects the wire output would
//! break this test loudly.

mod common;

use std::fs;
use std::path::Path;

use serde_json::Value;
use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::index::{self, OpenOptions};
use tome::mcp::prompts::PromptRegistry;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::WorkspaceName;

use common::{
    config_with_catalog, fabricate_models, lifecycle_paths, stub_embedder_seed, stub_reranker_seed,
    stub_summariser_seed,
};

fn build_deps<'a>(
    paths: &'a tome::paths::Paths,
    config: &'a tome::config::Config,
    embedder: &'a StubEmbedder,
    scope: &'a tome::workspace::Scope,
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

#[test]
fn prompts_list_payload_is_byte_stable_for_two_entry_fixture() {
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
        &[
            (
                "alpha",
                "---\nname: alpha\ndescription: First command.\n---\nbody\n",
            ),
            (
                "beta",
                "---\nname: beta\ndescription: Second command.\narguments: [target]\n---\nbody\n",
            ),
        ],
    );

    let embedder = StubEmbedder::new();
    let scope = tome::workspace::Scope(WorkspaceName::global());
    let deps = build_deps(&paths, &config, &embedder, &scope);
    let id: PluginId = "acme/p".parse().unwrap();

    // Seed the central DB's catalog enrolment + symlink the URL's
    // hashed cache_dir to the on-disk fixture so the registry's
    // resolve_plugin_dir_for_row walk succeeds. See
    // `tests/mcp_prompts.rs::stage_workspace_with` for the full
    // discipline; the steps are inlined here so this single-test
    // file stays self-contained. FF1: this must precede `lifecycle::enable`,
    // which now resolves the plugin dir from `workspace_catalogs`.
    {
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
    }

    lifecycle::enable(&id, &deps).expect("enable");

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
        PromptRegistry::build_for_workspace(&WorkspaceName::global(), &paths, &conn, false)
            .unwrap();

    let descriptors = registry.descriptors();
    let serialised = serde_json::to_value(&descriptors).expect("serialise");

    // Pinned JSON. The rmcp `Prompt` shape uses camelCase, but the
    // only camelCase fields in the post-cap output are absent / `None`
    // anyway, so the surviving keys are flat (`name`, `description`,
    // `arguments`, `required`).
    let expected: Value = serde_json::json!([
        {
            "name": "p__alpha",
            "description": "First command."
        },
        {
            "name": "p__beta",
            "description": "Second command.",
            "arguments": [
                { "name": "target", "required": true }
            ]
        }
    ]);

    assert_eq!(
        serialised,
        expected,
        "prompts/list wire-shape drift detected;\n  got:      {}\n  expected: {}",
        serde_json::to_string_pretty(&serialised).unwrap(),
        serde_json::to_string_pretty(&expected).unwrap(),
    );
}
