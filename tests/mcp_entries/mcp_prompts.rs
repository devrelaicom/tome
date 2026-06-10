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

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
use tome::workspace::{ResolvedScope, WorkspaceName};

use crate::common::{
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

    // FF1: the central-DB enrolment (+ the cache_dir symlink onto the on-disk
    // fixture) must exist BEFORE `lifecycle::enable`, which now resolves the
    // plugin dir from `workspace_catalogs` rather than the in-memory `Config`.
    seed_catalog_enrolment(&paths, &catalog_root, "acme");

    let embedder = StubEmbedder::new();
    let scope = tome::workspace::Scope(global());
    let deps = build_deps(&paths, &config, &embedder, &scope);
    let id: PluginId = "acme/plug".parse().unwrap();
    lifecycle::enable(&id, &deps).expect("enable plugin");

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
    let registry = PromptRegistry::build_for_workspace(&global(), &paths, &conn, false)
        .expect("build registry");
    // Phase 9 / US3: the reserved built-in is ALWAYS on, even with zero plugins
    // (positively asserted so a regression that dropped it can't hide behind the
    // filter below) — and an empty workspace yields no PLUGIN prompts.
    assert!(
        registry
            .descriptors()
            .iter()
            .any(|d| d.name == "add-tome-conversion-skill"),
        "reserved prompt is always-on even in an empty workspace",
    );
    let plugin: Vec<_> = registry
        .descriptors()
        .into_iter()
        .filter(|d| d.name != "add-tome-conversion-skill")
        .collect();
    assert!(plugin.is_empty());
    assert!(registry.collisions.is_empty());
}

#[test]
fn list_excludes_skills_by_default() {
    // Skills default to `user_invocable = false` — they MUST NOT appear
    // in `prompts/list`.
    let skill_body = "---\nname: only-skill\ndescription: just a skill\n---\nbody\n";
    let (_tmp, paths) = stage_workspace_with(&[("only-skill", skill_body)], &[]);

    let conn = open_index(&paths);
    let registry = PromptRegistry::build_for_workspace(&global(), &paths, &conn, false)
        .expect("build registry");
    // Phase 9 / US3: filter the always-on reserved built-in.
    let plugin: Vec<_> = registry
        .descriptors()
        .into_iter()
        .filter(|d| d.name != "add-tome-conversion-skill")
        .collect();
    assert!(
        plugin.is_empty(),
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
    let registry = PromptRegistry::build_for_workspace(&global(), &paths, &conn, false)
        .expect("build registry");
    let descriptors: Vec<_> = registry
        .descriptors()
        .into_iter()
        .filter(|d| d.name != "add-tome-conversion-skill")
        .collect();
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
    let registry = PromptRegistry::build_for_workspace(&global(), &paths, &conn, false)
        .expect("build registry");
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
    let registry = PromptRegistry::build_for_workspace(&global(), &paths, &conn, false)
        .expect("build registry");
    let descriptors: Vec<_> = registry
        .descriptors()
        .into_iter()
        .filter(|d| d.name != "add-tome-conversion-skill")
        .collect();
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
    let registry = PromptRegistry::build_for_workspace(&global(), &paths, &conn, false)
        .expect("build registry");
    let descriptors: Vec<_> = registry
        .descriptors()
        .into_iter()
        .filter(|d| d.name != "add-tome-conversion-skill")
        .collect();
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
    let registry = PromptRegistry::build_for_workspace(&global(), &paths, &conn, false)
        .expect("build registry");
    let descriptors = registry.descriptors();
    assert!(
        descriptors[0].arguments.is_none(),
        "entry with neither declared args nor $ARGUMENTS references must omit the argument schema; got {:?}",
        descriptors[0].arguments
    );
}

// ---------------------------------------------------------------------------
// prompts/get tests (US1.c).
//
// These exercise the real `handle_get` entry point — the rmcp
// `PromptRouter` machinery wraps it identically in production, but the
// test surface bypasses the router so we don't have to construct a
// synthetic `PromptContext` (which would require a `Server` instance +
// a `RequestContext` borrowed for the closure lifetime). `handle_get`
// is the silent compute path; the router is the emit wrapper.
//
// All tests share the `stage_workspace_with` fixture from the
// `prompts/list` section above; they additionally build an
// `Arc<McpState>` carrying the resolved `PromptRegistry` for the
// fixture's enabled entries.
// ---------------------------------------------------------------------------

/// Build an `Arc<McpState>` wrapping the fixture's `Paths` + a built
/// `PromptRegistry`. Mirrors `tests/mcp_server.rs::build_state` but
/// also resolves the registry (so the per-test prompts/get call has
/// a name to look up).
fn build_state_for_prompts(paths: &tome::paths::Paths) -> Arc<McpState> {
    let conn = open_index(paths);
    let registry = PromptRegistry::build_for_workspace(&global(), paths, &conn, false)
        .expect("build prompt registry");
    drop(conn);

    let embedder_entry = lookup("bge-small-en-v1.5").expect("registry has embedder");
    let reranker_entry = lookup("bge-reranker-base").expect("registry has reranker");
    let reranker: Arc<dyn Reranker> = Arc::new(StubReranker::new());

    // Resolve to the privileged `global` workspace so the registry's
    // `state.scope.scope.name()` lookup matches what
    // `PromptRegistry::build_for_workspace` was given. `global_fallback`
    // is the right shape — no project marker, no `--workspace` flag,
    // privileged-default `global` scope.
    let scope = ResolvedScope::global_fallback();

    Arc::new(McpState {
        embedder: Arc::new(StubEmbedder::new()),
        reranker: OnceCell::new_with(Some(reranker)),
        scope,
        paths: paths.clone(),
        embedder_entry,
        reranker_entry,
        prompt_registry: Arc::new(registry),
        host_harness: None,
    })
}

/// Convenience: invoke `prompts::handle_get` on a single-thread tokio
/// runtime. Returns the rendered body text on success.
fn invoke_get(
    state: Arc<McpState>,
    name: &str,
    arguments: Option<Map<String, Value>>,
) -> Result<String, rmcp::ErrorData> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let response = rt.block_on(prompts::handle_get(state, name.to_owned(), arguments))?;
    assert_eq!(response.messages.len(), 1, "single user-role message");
    let msg = &response.messages[0];
    match &msg.content {
        rmcp::model::PromptMessageContent::Text { text } => Ok(text.clone()),
        other => panic!("expected text content, got {other:?}"),
    }
}

#[test]
fn get_returns_unrendered_body_for_no_args_entry() {
    // F3 stub returns the body unchanged. The test asserts the
    // round-trip through handle_get → spawn_blocking → registry
    // lookup → render → wrap works end-to-end. US2+US3 will replace
    // the stub with real transforms; the wrapper shape stays the same.
    let cmd_body =
        "---\nname: bare\ndescription: A standalone command, no args.\n---\nDo a thing.\n";
    let (_tmp, paths) = stage_workspace_with(&[], &[("bare", cmd_body)]);
    let state = build_state_for_prompts(&paths);

    let text = invoke_get(state, "plug__bare", None).expect("prompts/get ok");
    // The frontmatter-stripped body — `parse_skill_frontmatter` returns
    // everything after the YAML closing `---` line.
    assert_eq!(text.trim(), "Do a thing.");
}

#[test]
fn get_returns_body_for_structured_named_args() {
    // 0-indexed positional refs per `contracts/substitution-engine.md`
    // § Stage 3. With declared `[component, from, to]` and caller
    // `{component: "frontend", from: "v1", to: "v2"}`:
    //   positional[0] = frontend, [1] = v1, [2] = v2
    // Body `$0 $1 $2` resolves to `frontend v1 v2`.
    let cmd_body = "---\nname: deploy\ndescription: Deploy.\narguments: [component, from, to]\n---\nRun a deploy for $0 from $1 to $2\n";
    let (_tmp, paths) = stage_workspace_with(&[], &[("deploy", cmd_body)]);
    let state = build_state_for_prompts(&paths);

    let mut args = Map::new();
    args.insert("component".into(), json!("frontend"));
    args.insert("from".into(), json!("v1"));
    args.insert("to".into(), json!("v2"));

    let text = invoke_get(state, "plug__deploy", Some(args)).expect("prompts/get ok");
    assert!(
        text.contains("Run a deploy for frontend from v1 to v2"),
        "Stage 3 positional substitution; got: {text:?}",
    );
}

#[test]
fn get_accepts_single_string_arg_via_catchall() {
    // No declared args; caller supplies `{ "args": "..." }` →
    // ArgumentValues::Single per FR-071. With declared empty, the
    // Stage-3 coercer treats the whole string as a single positional;
    // `$ARGUMENTS` resolves to positional.join(" ") = the whole string
    // per FR-042's whole-string convention.
    let cmd_body = "---\nname: fix\ndescription: Fix.\n---\nPlease fix $ARGUMENTS\n";
    let (_tmp, paths) = stage_workspace_with(&[], &[("fix", cmd_body)]);
    let state = build_state_for_prompts(&paths);

    let mut args = Map::new();
    args.insert("args".into(), json!("issue-123"));

    let text = invoke_get(state, "plug__fix", Some(args)).expect("prompts/get ok");
    assert!(
        text.contains("Please fix issue-123"),
        "$ARGUMENTS resolved via catch-all; got: {text:?}",
    );
}

#[test]
fn get_unknown_name_returns_prompt_not_found() {
    let cmd_body = "---\nname: real\ndescription: Real.\n---\nbody\n";
    let (_tmp, paths) = stage_workspace_with(&[], &[("real", cmd_body)]);
    let state = build_state_for_prompts(&paths);

    let err = invoke_get(state, "plug__does_not_exist", None)
        .expect_err("unknown prompt name must reject");
    let data = err.data.expect("structured error data");
    assert_eq!(
        data.get("code").and_then(|c| c.as_str()),
        Some("prompt_not_found"),
        "unknown prompt name → prompt_not_found; got {data}",
    );
    assert_eq!(
        data.get("name").and_then(|c| c.as_str()),
        Some("plug__does_not_exist"),
        "name round-trips in error envelope",
    );
}

#[test]
fn get_named_args_with_unknown_key_returns_prompt_argument_mismatch() {
    // Entry declares [a, b]; caller supplies `{c: ...}` → no match.
    let cmd_body = "---\nname: pair\ndescription: pair.\narguments: [a, b]\n---\nGot $1 $2\n";
    let (_tmp, paths) = stage_workspace_with(&[], &[("pair", cmd_body)]);
    let state = build_state_for_prompts(&paths);

    let mut args = Map::new();
    args.insert("c".into(), json!("nope"));

    let err =
        invoke_get(state, "plug__pair", Some(args)).expect_err("unknown arg name must reject");
    let data = err.data.expect("structured error data");
    assert_eq!(
        data.get("code").and_then(|c| c.as_str()),
        Some("prompt_argument_mismatch"),
        "unknown named key → prompt_argument_mismatch; got {data}",
    );
}

#[test]
fn get_no_declared_args_with_unknown_key_returns_prompt_argument_mismatch() {
    // No declared args; caller supplies a key OTHER than `args` → mismatch.
    let cmd_body = "---\nname: bare\ndescription: bare.\n---\nDo it\n";
    let (_tmp, paths) = stage_workspace_with(&[], &[("bare", cmd_body)]);
    let state = build_state_for_prompts(&paths);

    let mut args = Map::new();
    args.insert("not_args".into(), json!("oops"));

    let err = invoke_get(state, "plug__bare", Some(args))
        .expect_err("non-args key on no-declared-args entry must reject");
    let data = err.data.expect("structured error data");
    assert_eq!(
        data.get("code").and_then(|c| c.as_str()),
        Some("prompt_argument_mismatch"),
        "non-args key → prompt_argument_mismatch; got {data}",
    );
}

#[test]
fn get_response_description_uses_truncated_entry_description() {
    // The PromptGetResponse.description should mirror the
    // registry-cached (truncated) description so harnesses rendering
    // the response don't have to re-read prompts/list.
    let cmd_body = "---\nname: doc\ndescription: A short one.\n---\nbody\n";
    let (_tmp, paths) = stage_workspace_with(&[], &[("doc", cmd_body)]);
    let state = build_state_for_prompts(&paths);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let response = rt
        .block_on(prompts::handle_get(state, "plug__doc".into(), None))
        .expect("ok");
    assert_eq!(
        response.description.as_deref(),
        Some("A short one."),
        "response description carries the registry-truncated form",
    );
}

// ---- T-M3 (US1.d reviewer pass): registry-build degradation -----------
//
// `PromptRegistry::build_for_workspace` warns-and-skips entries whose
// body file went missing on disk between enable and registry build
// (catalog cache evicted, manual deletion) or whose frontmatter parses
// fail. The rest of the registry must still build so a single bad
// entry doesn't take prompts/list down. These tests pin both warn-skip
// branches.

#[test]
fn registry_build_skips_entry_with_missing_body_file() {
    // Two commands enabled; delete one body file off disk; assert
    // registry builds with only the surviving prompt.
    let cmd1 = "---\nname: alive\ndescription: still here.\n---\nbody\n";
    let cmd2 = "---\nname: gone\ndescription: about to vanish.\n---\nbody\n";
    let (tmp, paths) = stage_workspace_with(&[], &[("alive", cmd1), ("gone", cmd2)]);

    // Delete one of the on-disk command bodies. The `stage_workspace_with`
    // fixture wrote it to `<catalog_root>/plug/commands/gone.md`.
    let catalog_root = tmp.path().join("catalog");
    let gone_path = catalog_root.join("plug").join("commands").join("gone.md");
    assert!(gone_path.exists(), "fixture wrote the body file");
    std::fs::remove_file(&gone_path).expect("delete body file");

    let conn = open_index(&paths);
    let registry = PromptRegistry::build_for_workspace(&global(), &paths, &conn, false)
        .expect("registry build must succeed when one body file is missing");

    let names: Vec<String> = registry
        .descriptors()
        .iter()
        .map(|p| p.name.clone())
        .filter(|n| n != "add-tome-conversion-skill")
        .collect();
    assert_eq!(
        names,
        vec!["plug__alive"],
        "missing body must warn-and-skip; surviving entry must remain",
    );
}

#[test]
fn registry_build_skips_entry_with_malformed_frontmatter() {
    // Two commands enabled; corrupt one's frontmatter after enable;
    // assert registry builds with only the surviving prompt.
    let cmd1 = "---\nname: alive\ndescription: still here.\n---\nbody\n";
    let cmd2 = "---\nname: malformed\ndescription: pristine.\n---\nbody\n";
    let (tmp, paths) = stage_workspace_with(&[], &[("alive", cmd1), ("malformed", cmd2)]);

    // Replace the body of `malformed.md` with content that fails the
    // frontmatter parse (no opening `---`).
    let catalog_root = tmp.path().join("catalog");
    let bad_path = catalog_root
        .join("plug")
        .join("commands")
        .join("malformed.md");
    std::fs::write(&bad_path, "no frontmatter delimiters here\n").expect("rewrite body");

    let conn = open_index(&paths);
    let registry = PromptRegistry::build_for_workspace(&global(), &paths, &conn, false)
        .expect("registry build must succeed when one frontmatter is malformed");

    let names: Vec<String> = registry
        .descriptors()
        .iter()
        .map(|p| p.name.clone())
        .filter(|n| n != "add-tome-conversion-skill")
        .collect();
    assert_eq!(
        names,
        vec!["plug__alive"],
        "malformed frontmatter must warn-and-skip; surviving entry must remain",
    );
}

#[test]
fn description_truncated_at_300_chars_with_ellipsis() {
    // Build a 350-char description and confirm the registry caps it.
    let long: String = "x".repeat(350);
    let cmd_body = format!("---\nname: chatty\ndescription: {long}\n---\nbody\n");
    let (_tmp, paths) = stage_workspace_with(&[], &[("chatty", cmd_body.as_str())]);

    let conn = open_index(&paths);
    let registry = PromptRegistry::build_for_workspace(&global(), &paths, &conn, false)
        .expect("build registry");
    let desc = registry
        .descriptors()
        .into_iter()
        .find(|d| d.name == "plug__chatty")
        .expect("chatty prompt present")
        .description
        .clone()
        .expect("description present");
    assert_eq!(desc.chars().count(), 300);
    assert!(
        desc.ends_with('\u{2026}'),
        "truncated description ends with `…`"
    );
}
