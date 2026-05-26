//! Phase 5 / US1.b — MCP prompts surface end-to-end at the library API.
//!
//! Drives `PromptRegistry::build_for_workspace` against fixtures laid
//! out on disk + indexed via `lifecycle::enable` (StubEmbedder so the
//! test stays library-API only, no ONNX models needed). The CLI binary
//! is not invoked here — that path is exercised by the protocol-level
//! tests deferred to T088 / SC-001 manual verification.
//!
//! Covers `contracts/mcp-prompts.md`:
//! - Empty workspace → empty prompt set.
//! - Skill-only entries (`user_invocable = false` by default) are
//!   excluded.
//! - Commands (`user_invocable = true` by default) are surfaced.
//! - Both kinds surface together when the skill opts in via
//!   `user-invocable: true`.
//! - Argument-schema derivation: named args become required strings;
//!   no-args entries with `$ARGUMENTS` get the catch-all; entries with
//!   neither get `arguments: None`.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

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

// ---------------------------------------------------------------------------
// Fixture helpers.
// ---------------------------------------------------------------------------

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

fn write_plugin(
    catalog_root: &Path,
    plugin_name: &str,
    skills: &[(&str, &str)],
    commands: &[(&str, &str)],
) -> PathBuf {
    let plugin_dir = catalog_root.join(plugin_name);
    fs::create_dir_all(plugin_dir.join(".claude-plugin")).unwrap();
    fs::write(
        plugin_dir.join(".claude-plugin").join("plugin.json"),
        format!(r#"{{"name": "{plugin_name}", "version": "1.0.0"}}"#),
    )
    .unwrap();
    for (name, body) in skills {
        let dir = plugin_dir.join("skills").join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), body).unwrap();
    }
    if !commands.is_empty() {
        let cmd_dir = plugin_dir.join("commands");
        fs::create_dir_all(&cmd_dir).unwrap();
        for (name, body) in commands {
            fs::write(cmd_dir.join(format!("{name}.md")), body).unwrap();
        }
    }
    plugin_dir
}

fn open_index(paths: &tome::paths::Paths) -> rusqlite::Connection {
    index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .expect("open index db")
}

fn global() -> WorkspaceName {
    WorkspaceName::global()
}

/// Stage a workspace with a single plugin enabled. Returns the temp dir
/// (must outlive the test) plus `Paths` rooted in it.
///
/// The fixture wires up TWO sources of catalog truth:
/// 1. An in-memory `Config` for `lifecycle::enable`'s `resolve_plugin_dir`.
/// 2. A row in the central DB's `workspace_catalogs` table for the
///    `PromptRegistry`'s `resolve_catalog_path` lookup.
fn stage_workspace_with(
    skills: &[(&str, &str)],
    commands: &[(&str, &str)],
) -> (TempDir, tome::paths::Paths) {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = tmp.path().join("catalog");
    fs::create_dir_all(&catalog_root).unwrap();
    let config = config_with_catalog("acme", &catalog_root);
    write_plugin(&catalog_root, "plug", skills, commands);

    let embedder = StubEmbedder::new();
    let scope = tome::workspace::Scope(global());
    let deps = build_deps(&paths, &config, &embedder, &scope);
    let id: PluginId = "acme/plug".parse().unwrap();
    lifecycle::enable(&id, &deps).expect("enable plugin");

    // Seed the central DB's catalog enrolment so the
    // PromptRegistry's resolve_catalog_path lookup finds a URL it can
    // hash into `paths.cache_dir_for(url)`. The URL is constructed to
    // hash into the same on-disk directory that hosts the catalog
    // fixture, so the registry's `read_catalog_manifest(catalog_path)`
    // + `catalog_path.join(plugin)` walk hits the real files.
    seed_catalog_enrolment(&paths, &catalog_root, "acme");

    (tmp, paths)
}

/// Insert a `workspace_catalogs` row for the privileged `global`
/// workspace pointing at `catalog_root`. The URL is set to the
/// `file://` form of `catalog_root` and a symlink (or copy) is
/// arranged so `paths.cache_dir_for(url)` resolves into the real
/// catalog directory the lifecycle fixture wrote into.
fn seed_catalog_enrolment(paths: &tome::paths::Paths, catalog_root: &Path, catalog_name: &str) {
    let url = format!("file://{}", catalog_root.display());
    let conn = open_index(paths);
    tome::index::workspace_catalogs::insert(&conn, "global", catalog_name, &url, "main")
        .expect("seed workspace_catalogs");
    drop(conn);

    // `paths.cache_dir_for(url)` is `<root>/catalogs/<sha256(url)>` —
    // create a symlink (or copy) from that hashed dir to the
    // catalog_root the fixture wrote so the registry's manifest read
    // succeeds.
    let cache_dir = paths.cache_dir_for(&url);
    if let Some(parent) = cache_dir.parent() {
        fs::create_dir_all(parent).expect("create catalogs parent");
    }
    if !cache_dir.exists() {
        #[cfg(unix)]
        std::os::unix::fs::symlink(catalog_root, &cache_dir).expect("symlink catalog cache");
        #[cfg(not(unix))]
        {
            // Windows fallback: recursive copy. Tests are macOS / Linux
            // in practice, so this branch is mostly defensive.
            fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
                fs::create_dir_all(dst)?;
                for entry in fs::read_dir(src)? {
                    let entry = entry?;
                    let to = dst.join(entry.file_name());
                    if entry.file_type()?.is_dir() {
                        copy_dir(&entry.path(), &to)?;
                    } else {
                        fs::copy(entry.path(), &to)?;
                    }
                }
                Ok(())
            }
            copy_dir(catalog_root, &cache_dir).expect("copy catalog cache");
        }
    }
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[test]
fn list_returns_empty_for_empty_workspace() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    fs::create_dir_all(&paths.root).unwrap();
    // Bootstrap an empty index DB.
    let _ = open_index(&paths);

    let conn = open_index(&paths);
    let registry =
        PromptRegistry::build_for_workspace(&global(), &paths, &conn).expect("build registry");
    assert!(registry.by_name.is_empty());
    assert!(registry.collisions.is_empty());
    assert!(registry.descriptors().is_empty());
}

#[test]
fn list_excludes_skills_by_default() {
    // Skills default to `user_invocable = false` — they MUST NOT appear
    // in `prompts/list`.
    let skill_body = "---\nname: only-skill\ndescription: just a skill\n---\nbody\n";
    let (_tmp, paths) = stage_workspace_with(&[("only-skill", skill_body)], &[]);

    let conn = open_index(&paths);
    let registry =
        PromptRegistry::build_for_workspace(&global(), &paths, &conn).expect("build registry");
    assert!(
        registry.by_name.is_empty(),
        "skills with default user_invocable=false must be excluded; got {:?}",
        registry.by_name.keys().collect::<Vec<_>>()
    );
}

#[test]
fn list_includes_commands_by_default() {
    let cmd_body =
        "---\nname: fix-issue\ndescription: Fix a GitHub issue.\n---\nGo fix issue $ARGUMENTS\n";
    let (_tmp, paths) = stage_workspace_with(&[], &[("fix-issue", cmd_body)]);

    let conn = open_index(&paths);
    let registry =
        PromptRegistry::build_for_workspace(&global(), &paths, &conn).expect("build registry");
    let descriptors = registry.descriptors();
    assert_eq!(descriptors.len(), 1, "one command becomes one prompt");
    assert_eq!(descriptors[0].name, "plug__fix-issue");
    assert_eq!(
        descriptors[0].description.as_deref(),
        Some("Fix a GitHub issue.")
    );
}

#[test]
fn list_includes_both_kinds_when_skill_opts_in() {
    let skill_body = "---\nname: also-invocable\ndescription: surfaced for prompts.\nuser-invocable: true\n---\nbody\n";
    let cmd_body = "---\nname: fix-issue\ndescription: Fix something.\n---\nGo fix $ARGUMENTS\n";
    let (_tmp, paths) = stage_workspace_with(
        &[("also-invocable", skill_body)],
        &[("fix-issue", cmd_body)],
    );

    let conn = open_index(&paths);
    let registry =
        PromptRegistry::build_for_workspace(&global(), &paths, &conn).expect("build registry");
    let names: Vec<String> = registry
        .descriptors()
        .iter()
        .map(|p| p.name.clone())
        .collect();
    assert!(names.contains(&"plug__also-invocable".to_owned()));
    assert!(names.contains(&"plug__fix-issue".to_owned()));
}

#[test]
fn named_arguments_become_required_string_array() {
    let cmd_body = "---
name: deploy
description: Deploy a component from one version to another.
arguments: [component, from, to]
---
Run a deploy for $1 from $2 to $3
";
    let (_tmp, paths) = stage_workspace_with(&[], &[("deploy", cmd_body)]);

    let conn = open_index(&paths);
    let registry =
        PromptRegistry::build_for_workspace(&global(), &paths, &conn).expect("build registry");
    let descriptors = registry.descriptors();
    assert_eq!(descriptors.len(), 1);
    let args = descriptors[0]
        .arguments
        .as_ref()
        .expect("named args expose argument schema");
    let names: Vec<&str> = args.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, vec!["component", "from", "to"]);
    for a in args {
        assert_eq!(
            a.required,
            Some(true),
            "named arg `{}` must be required per FR-070",
            a.name
        );
    }
}

#[test]
fn no_named_args_with_arguments_in_body_becomes_optional_catchall() {
    let cmd_body = "---
name: fix-issue
description: Fix a GitHub issue.
argument-hint: GitHub issue number or URL
---
Please fix issue $ARGUMENTS
";
    let (_tmp, paths) = stage_workspace_with(&[], &[("fix-issue", cmd_body)]);

    let conn = open_index(&paths);
    let registry =
        PromptRegistry::build_for_workspace(&global(), &paths, &conn).expect("build registry");
    let descriptors = registry.descriptors();
    let args = descriptors[0]
        .arguments
        .as_ref()
        .expect("catch-all `args` exposed when body references $ARGUMENTS");
    assert_eq!(args.len(), 1);
    assert_eq!(args[0].name, "args");
    assert_eq!(args[0].required, Some(false));
    assert_eq!(
        args[0].description.as_deref(),
        Some("GitHub issue number or URL"),
        "argument-hint frontmatter populates the catch-all description"
    );
}

#[test]
fn no_args_and_no_arguments_reference_yields_arguments_none() {
    let cmd_body = "---
name: bare
description: A standalone command, no args.
---
Do a thing.
";
    let (_tmp, paths) = stage_workspace_with(&[], &[("bare", cmd_body)]);

    let conn = open_index(&paths);
    let registry =
        PromptRegistry::build_for_workspace(&global(), &paths, &conn).expect("build registry");
    let descriptors = registry.descriptors();
    assert!(
        descriptors[0].arguments.is_none(),
        "entry with neither declared args nor $ARGUMENTS references must omit the argument schema; got {:?}",
        descriptors[0].arguments
    );
}

#[test]
fn description_truncated_at_300_chars_with_ellipsis() {
    // Build a 350-char description and confirm the registry caps it.
    let long: String = "x".repeat(350);
    let cmd_body = format!("---\nname: chatty\ndescription: {long}\n---\nbody\n");
    let (_tmp, paths) = stage_workspace_with(&[], &[("chatty", cmd_body.as_str())]);

    let conn = open_index(&paths);
    let registry =
        PromptRegistry::build_for_workspace(&global(), &paths, &conn).expect("build registry");
    let desc = registry.descriptors()[0]
        .description
        .clone()
        .expect("description present");
    assert_eq!(desc.chars().count(), 300);
    assert!(
        desc.ends_with('\u{2026}'),
        "truncated description ends with `…`"
    );
}
