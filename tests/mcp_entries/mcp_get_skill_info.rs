//! Consolidated `get_skill` metadata-only mode end-to-end at the library API.
//! #497 (was the standalone `get_skill_info` tool).
//!
//! Drives the real handler against a staged workspace + indexed plugin using
//! the StubEmbedder (no ONNX models needed). Every call passes
//! `metadata_only: true` — the middle-tier introspection that returns
//! description + when_to_use + a capped resource enumeration WITHOUT reading
//! the body.
//!
//! Covers the former `get_skill_info` contract behaviours:
//!
//! - Skill-kind entry returns full description + when_to_use + resources.
//! - Command-kind entry omits the structured `resources` key (FR-083).
//! - Per-directory cap of 5 + `"and N more"` sentinel.
//! - Subdir cap: per-subdir, NOT just top-level.
//! - Default `kind` parameter selects `skill`.
//! - Same name across both kinds → `kind` disambiguator selects.
//! - Not-found surfaces the three-code surface (`unknown_catalog` /
//!   `unknown_plugin` / `unknown_skill`) with the `available` payload.
//! - `*` wildcard resolution (one / many / zero matches).
//! - Alphabetical ordering by basename.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tempfile::TempDir;
use tokio::sync::OnceCell;
use tome::embedding::Reranker;
use tome::embedding::registry::lookup;
use tome::embedding::stub::{StubEmbedder, StubReranker};
use tome::index::{self, OpenOptions};
use tome::mcp::prompts::PromptRegistry;
use tome::mcp::state::McpState;
use tome::mcp::tools::get_skill::{self, Input};
use tome::plugin::PluginId;
use tome::plugin::identity::EntryKind;
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

fn open_index(paths: &tome::paths::Paths) -> rusqlite::Connection {
    index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
            profile: None,
        },
    )
    .expect("open index db")
}

fn global() -> WorkspaceName {
    WorkspaceName::global()
}

/// Bundle of files to write into a skill's directory: `(relative_path, body)`.
type SkillFile<'a> = (&'a str, &'a str);

/// Build a metadata-only `get_skill::Input`.
fn meta_input(catalog: &str, plugin: &str, name: &str, kind: EntryKind) -> Input {
    Input {
        catalog: Some(catalog.into()),
        plugin: Some(plugin.into()),
        name: Some(name.into()),
        uri: None,
        kind: Some(kind),
        metadata_only: true,
        raw: false,
        include_resource_bodies: false,
    }
}

/// Stage a workspace with one plugin enabled.
fn stage_workspace(
    skills: &[(&str, &str, &[SkillFile<'_>])],
    commands: &[(&str, &str)],
) -> (TempDir, tome::paths::Paths) {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = tmp.path().join("catalog");
    fs::create_dir_all(&catalog_root).unwrap();
    let config = config_with_catalog("acme", &catalog_root);

    let plugin_dir = catalog_root.join("plug");
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
        r#"{"name": "plug", "version": "1.0.0"}"#,
    )
    .unwrap();

    for (name, body, extras) in skills {
        let dir = plugin_dir.join("skills").join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), body).unwrap();
        for (rel, content) in *extras {
            let target = dir.join(rel);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(target, content).unwrap();
        }
    }
    if !commands.is_empty() {
        let cmd_dir = plugin_dir.join("commands");
        fs::create_dir_all(&cmd_dir).unwrap();
        for (name, body) in commands {
            fs::write(cmd_dir.join(format!("{name}.md")), body).unwrap();
        }
    }

    let embedder = StubEmbedder::new();
    let scope = tome::workspace::Scope(global());
    let deps = build_deps(&paths, &config, &embedder, &scope);
    let id: PluginId = "acme/plug".parse().unwrap();
    seed_catalog_enrolment(&paths, &catalog_root, "acme");
    lifecycle::enable(&id, &deps).expect("enable plugin");
    (tmp, paths)
}

fn seed_catalog_enrolment(paths: &tome::paths::Paths, catalog_root: &Path, catalog_name: &str) {
    let url = format!("file://{}", catalog_root.display());
    let conn = open_index(paths);
    tome::index::workspace_catalogs::insert(&conn, "global", catalog_name, &url, "main")
        .expect("seed workspace_catalogs");
    drop(conn);

    let cache_dir = paths.cache_dir_for(&url);
    if let Some(parent) = cache_dir.parent() {
        fs::create_dir_all(parent).expect("create catalogs parent");
    }
    if !cache_dir.exists() {
        #[cfg(unix)]
        std::os::unix::fs::symlink(catalog_root, &cache_dir).expect("symlink catalog cache");
        #[cfg(not(unix))]
        {
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

fn build_state(paths: &tome::paths::Paths) -> Arc<McpState> {
    let embedder_entry = lookup("bge-small-en-v1.5").expect("registry has embedder");
    let reranker_entry = lookup("bge-reranker-base").expect("registry has reranker");
    let reranker: Arc<dyn Reranker> = Arc::new(StubReranker::new());
    Arc::new(McpState {
        embedder: Arc::new(StubEmbedder::new()),
        reranker: OnceCell::new_with(Some(reranker)),
        scope: ResolvedScope::global_fallback(),
        paths: paths.clone(),
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

fn invoke(state: Arc<McpState>, input: Input) -> Result<get_skill::Output, rmcp::ErrorData> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(get_skill::handle(state, input))
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[test]
fn skill_info_includes_resources() {
    let body = "---
name: with-resources
description: A skill that ships sibling files.
when_to_use: When the user mentions resource enumeration.
---
body
";
    let extras: Vec<SkillFile<'_>> = vec![
        ("config.json", "{}"),
        ("examples/basic.ts", "// basic"),
        ("examples/advanced.ts", "// advanced"),
        ("scripts/run.sh", "#!/bin/sh"),
    ];
    let (_tmp, paths) = stage_workspace(&[("with-resources", body, &extras)], &[]);
    let state = build_state(&paths);

    let info = invoke(
        state,
        meta_input("acme", "plug", "with-resources", EntryKind::Skill),
    )
    .expect("get_skill metadata ok");

    assert_eq!(info.catalog, "acme");
    assert_eq!(info.plugin, "plug");
    assert_eq!(info.name, "with-resources");
    assert!(matches!(info.kind, EntryKind::Skill));
    assert_eq!(
        info.description.as_deref(),
        Some("A skill that ships sibling files.")
    );
    // Metadata mode does not read the body.
    assert!(
        info.content.is_none(),
        "metadata mode must not fetch the body"
    );
    // `when_to_use` is exposed via the tri-state wire — round-trip via JSON.
    let json = serde_json::to_value(&info).expect("serialise");
    assert_eq!(
        json.get("when_to_use").and_then(|v| v.as_str()),
        Some("When the user mentions resource enumeration."),
    );
    assert_eq!(info.plugin_version.as_deref(), Some("1.0.0"));
    assert_eq!(
        info.user_invocable,
        Some(false),
        "skills default user_invocable=false"
    );
    assert!(info.path.ends_with("SKILL.md"));

    let res = info.resources.expect("skill MUST carry resources field");
    assert_eq!(res.files.len(), 1);
    assert!(res.files[0].ends_with("config.json"));

    let keys: Vec<&str> = res.directories.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["examples", "scripts"]);

    let examples = res.directories.get("examples").unwrap();
    assert_eq!(examples.len(), 2);
    assert!(examples[0].ends_with("advanced.ts"));
    assert!(examples[1].ends_with("basic.ts"));

    let scripts = res.directories.get("scripts").unwrap();
    assert_eq!(scripts.len(), 1);
    assert!(scripts[0].ends_with("run.sh"));
}

#[test]
fn command_info_omits_resources() {
    let cmd_body =
        "---\nname: fix-issue\ndescription: Fix a GitHub issue.\n---\nGo fix $ARGUMENTS\n";
    let (_tmp, paths) = stage_workspace(&[], &[("fix-issue", cmd_body)]);
    let state = build_state(&paths);

    let info = invoke(
        state,
        meta_input("acme", "plug", "fix-issue", EntryKind::Command),
    )
    .expect("get_skill metadata ok for command");

    assert_eq!(info.name, "fix-issue");
    assert!(matches!(info.kind, EntryKind::Command));
    assert_eq!(info.description.as_deref(), Some("Fix a GitHub issue."));
    assert_eq!(
        info.user_invocable,
        Some(true),
        "commands default user_invocable=true per resolved-defaults table"
    );
    assert!(
        info.resources.is_none(),
        "FR-083: command-kind MUST omit the resources field; got Some(...)"
    );

    // Serialise + assert the JSON shape physically lacks the structured
    // `resources` object key.
    let json = serde_json::to_value(&info).expect("serialise");
    let obj = json.as_object().expect("object");
    assert!(
        obj.get("resources").is_none(),
        "command metadata JSON must not include `resources`, got: {}",
        json
    );
    // when_to_use serialises as null for a command without guidance.
    assert!(
        json.get("when_to_use")
            .map(|v| v.is_null())
            .unwrap_or(false)
    );
}

#[test]
fn heavy_directory_capped_with_sentinel() {
    let mut extras: Vec<(String, String)> = Vec::new();
    for i in 0..7 {
        extras.push((format!("file-{i:02}.txt"), "x".to_owned()));
    }
    for i in 0..7 {
        extras.push((format!("scripts/step-{i:02}.sh"), "x".to_owned()));
    }
    let extras_ref: Vec<SkillFile<'_>> = extras
        .iter()
        .map(|(p, b)| (p.as_str(), b.as_str()))
        .collect();
    let body = "---\nname: heavy\ndescription: many files.\n---\nbody\n";
    let (_tmp, paths) = stage_workspace(&[("heavy", body, &extras_ref)], &[]);
    let state = build_state(&paths);

    let info = invoke(state, meta_input("acme", "plug", "heavy", EntryKind::Skill)).expect("ok");

    let res = info.resources.expect("skill resources");
    assert_eq!(res.files.len(), 6, "5 files + 1 sentinel");
    assert_eq!(res.files[5], "and 2 more");

    let scripts = res.directories.get("scripts").unwrap();
    assert_eq!(scripts.len(), 6);
    assert_eq!(scripts[5], "and 2 more");
}

#[test]
fn default_kind_is_skill() {
    let skill_body = "---\nname: shared-name\ndescription: a skill.\n---\nbody\n";
    let (_tmp, paths) = stage_workspace(&[("shared-name", skill_body, &[])], &[]);
    let state = build_state(&paths);

    // Construct a JSON request without `kind` (but WITH metadata_only) and
    // round-trip via the tool's deserialiser.
    let raw = serde_json::json!({
        "catalog": "acme",
        "plugin": "plug",
        "name": "shared-name",
        "metadata_only": true,
    });
    let input: Input = serde_json::from_value(raw).expect("deserialise default kind");
    assert!(
        input.kind.is_none(),
        "kind omitted from JSON must deserialise to None (defaulted to Skill by `into_request`)",
    );

    let info = invoke(state, input).expect("ok");
    assert!(matches!(info.kind, EntryKind::Skill));
    assert_eq!(info.description.as_deref(), Some("a skill."));
}

#[test]
fn kind_disambiguates_same_name_across_kinds() {
    let skill_body = "---
name: deploy
description: SKILL deploy.
user-invocable: true
---
skill body
";
    let cmd_body = "---\nname: deploy\ndescription: COMMAND deploy.\n---\ncommand body\n";
    let (_tmp, paths) = stage_workspace(&[("deploy", skill_body, &[])], &[("deploy", cmd_body)]);
    let state = build_state(&paths);

    let skill_info = invoke(
        state.clone(),
        meta_input("acme", "plug", "deploy", EntryKind::Skill),
    )
    .expect("skill lookup ok");
    assert_eq!(skill_info.description.as_deref(), Some("SKILL deploy."));
    assert!(matches!(skill_info.kind, EntryKind::Skill));

    let cmd_info = invoke(
        state,
        meta_input("acme", "plug", "deploy", EntryKind::Command),
    )
    .expect("command lookup ok");
    assert_eq!(cmd_info.description.as_deref(), Some("COMMAND deploy."));
    assert!(matches!(cmd_info.kind, EntryKind::Command));
    assert!(cmd_info.resources.is_none());
}

#[test]
fn unknown_entry_surfaces_unknown_skill() {
    let real = "---\nname: real\ndescription: ok.\n---\nbody\n";
    let cmd = "---\nname: run-it\ndescription: a command.\n---\ndo it\n";
    let (_tmp, paths) = stage_workspace(&[("real", real, &[])], &[("run-it", cmd)]);
    let state = build_state(&paths);

    let err = invoke(
        state,
        meta_input("acme", "plug", "does-not-exist", EntryKind::Skill),
    )
    .expect_err("unknown entry must reject");

    let data = err.data.expect("structured error data");
    assert_eq!(
        data.get("code").and_then(|c| c.as_str()),
        Some("unknown_skill"),
        "expected unknown_skill code, got: {}",
        data,
    );
    assert_eq!(data.get("catalog").and_then(|c| c.as_str()), Some("acme"));
    assert_eq!(data.get("plugin").and_then(|c| c.as_str()), Some("plug"));
    assert_eq!(
        data.get("name").and_then(|c| c.as_str()),
        Some("does-not-exist"),
    );
    assert_eq!(
        err.message,
        "skill `acme/plug/does-not-exist` is not enabled in the resolved scope",
    );

    let available = data
        .get("available")
        .and_then(|a| a.as_array())
        .expect("unknown_skill data must carry an `available` array");
    let pairs: Vec<(String, String)> = available
        .iter()
        .map(|e| {
            (
                e.get("name").and_then(|n| n.as_str()).unwrap().to_owned(),
                e.get("kind").and_then(|k| k.as_str()).unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("run-it".to_owned(), "command".to_owned()),
            ("real".to_owned(), "skill".to_owned()),
        ],
        "available must list every enabled (name, kind) for the plugin",
    );
}

#[test]
fn unknown_catalog_surfaces_unknown_catalog() {
    let body = "---\nname: real\ndescription: ok.\n---\nbody\n";
    let (_tmp, paths) = stage_workspace(&[("real", body, &[])], &[]);
    let state = build_state(&paths);

    let err = invoke(
        state,
        meta_input("nonexistent", "plug", "real", EntryKind::Skill),
    )
    .expect_err("unknown catalog must reject");

    let data = err.data.expect("structured error data");
    assert_eq!(
        data.get("code").and_then(|c| c.as_str()),
        Some("unknown_catalog"),
        "expected unknown_catalog code, got: {data}",
    );
    assert_eq!(
        data.get("catalog").and_then(|c| c.as_str()),
        Some("nonexistent"),
    );
    assert_eq!(
        err.message,
        "catalog `nonexistent` is not enabled in the resolved scope",
    );
}

#[test]
fn unknown_plugin_surfaces_unknown_plugin() {
    let body = "---\nname: real\ndescription: ok.\n---\nbody\n";
    let (_tmp, paths) = stage_workspace(&[("real", body, &[])], &[]);
    let state = build_state(&paths);

    let err = invoke(
        state,
        meta_input("acme", "no-such-plugin", "real", EntryKind::Skill),
    )
    .expect_err("unknown plugin must reject");

    let data = err.data.expect("structured error data");
    assert_eq!(
        data.get("code").and_then(|c| c.as_str()),
        Some("unknown_plugin"),
        "expected unknown_plugin code, got: {data}",
    );
    assert_eq!(data.get("catalog").and_then(|c| c.as_str()), Some("acme"));
    assert_eq!(
        data.get("plugin").and_then(|c| c.as_str()),
        Some("no-such-plugin"),
    );
    assert_eq!(
        err.message,
        "plugin `acme/no-such-plugin` is not enabled in the resolved scope",
    );
}

#[test]
fn empty_field_validation_rejects() {
    let body = "---\nname: real\ndescription: ok.\n---\nbody\n";
    let (_tmp, paths) = stage_workspace(&[("real", body, &[])], &[]);
    let state = build_state(&paths);

    let err = invoke(state, meta_input("", "plug", "real", EntryKind::Skill))
        .expect_err("empty catalog must reject");
    assert!(
        err.message.contains("non-empty"),
        "expected empty-field rejection, got: {}",
        err.message,
    );
}

#[test]
fn alphabetical_ordering_independent_of_creation_order() {
    let extras: Vec<SkillFile<'_>> =
        vec![("zebra.txt", "z"), ("mango.txt", "m"), ("apple.txt", "a")];
    let body = "---\nname: ordered\ndescription: ordering check.\n---\nbody\n";
    let (_tmp, paths) = stage_workspace(&[("ordered", body, &extras)], &[]);
    let state = build_state(&paths);

    let info = invoke(
        state,
        meta_input("acme", "plug", "ordered", EntryKind::Skill),
    )
    .expect("ok");

    let res = info.resources.expect("resources present");
    assert_eq!(res.files.len(), 3);
    assert!(res.files[0].ends_with("apple.txt"));
    assert!(res.files[1].ends_with("mango.txt"));
    assert!(res.files[2].ends_with("zebra.txt"));
}

// ---------------------------------------------------------------------------
// Wildcard `name` resolution (metadata-only mode).
// ---------------------------------------------------------------------------

#[test]
fn glob_name_matching_one_resolves() {
    let body = "---\nname: compact-circuits\ndescription: circuit skill.\n---\nbody\n";
    let other = "---\nname: unrelated\ndescription: other skill.\n---\nbody\n";
    let (_tmp, paths) = stage_workspace(
        &[("compact-circuits", body, &[]), ("unrelated", other, &[])],
        &[],
    );
    let state = build_state(&paths);

    let info = invoke(
        state,
        meta_input("acme", "plug", "compact-*", EntryKind::Skill),
    )
    .expect("glob matching one entry must resolve");

    assert_eq!(
        info.name, "compact-circuits",
        "the response reports the RESOLVED concrete name, not the glob pattern",
    );
    assert!(matches!(info.kind, EntryKind::Skill));
    assert_eq!(info.description.as_deref(), Some("circuit skill."));
    assert!(info.path.ends_with("SKILL.md"));
}

#[test]
fn glob_name_matching_many_is_ambiguous() {
    let a = "---\nname: compact-lint\ndescription: lint.\n---\nbody\n";
    let b = "---\nname: compact-fmt\ndescription: fmt.\n---\nbody\n";
    let (_tmp, paths) = stage_workspace(&[("compact-lint", a, &[]), ("compact-fmt", b, &[])], &[]);
    let state = build_state(&paths);

    let err = invoke(
        state,
        meta_input("acme", "plug", "compact-*", EntryKind::Skill),
    )
    .expect_err("ambiguous glob must reject");

    let data = err.data.expect("structured error data");
    assert_eq!(
        data.get("code").and_then(|c| c.as_str()),
        Some("ambiguous_name"),
        "ambiguous glob must carry the `ambiguous_name` code; got: {data}",
    );
    assert_eq!(data.get("catalog").and_then(|c| c.as_str()), Some("acme"));
    assert_eq!(data.get("plugin").and_then(|c| c.as_str()), Some("plug"));
    assert_eq!(data.get("name").and_then(|c| c.as_str()), Some("compact-*"));

    let candidates = data
        .get("candidates")
        .and_then(|c| c.as_array())
        .expect("ambiguous error must list `candidates`");
    let names: Vec<&str> = candidates
        .iter()
        .filter_map(|e| e.get("name").and_then(|n| n.as_str()))
        .collect();
    assert!(names.contains(&"compact-lint"));
    assert!(names.contains(&"compact-fmt"));
    assert_eq!(names.len(), 2, "both matches listed; got: {candidates:?}");
    for c in candidates {
        assert_eq!(c.get("kind").and_then(|k| k.as_str()), Some("skill"));
    }
}

#[test]
fn glob_name_matching_zero_is_unknown_skill_with_available() {
    let body = "---\nname: real\ndescription: ok.\n---\nbody\n";
    let (_tmp, paths) = stage_workspace(&[("real", body, &[])], &[]);
    let state = build_state(&paths);

    let err = invoke(
        state,
        meta_input("acme", "plug", "nomatch-*", EntryKind::Skill),
    )
    .expect_err("zero-match glob must reject as unknown_skill");

    let data = err.data.expect("structured error data");
    assert_eq!(
        data.get("code").and_then(|c| c.as_str()),
        Some("unknown_skill"),
        "zero-match glob is an entry-not-found; got: {data}",
    );
    assert_eq!(data.get("name").and_then(|c| c.as_str()), Some("nomatch-*"));

    let available = data
        .get("available")
        .and_then(|a| a.as_array())
        .expect("zero-match glob must carry `available`");
    let names: Vec<&str> = available
        .iter()
        .filter_map(|e| e.get("name").and_then(|n| n.as_str()))
        .collect();
    assert_eq!(names, vec!["real"], "available lists the one enabled skill");
}

#[test]
fn glob_name_respects_kind_filter() {
    let skill = "---\nname: deploy\ndescription: SKILL deploy.\nuser-invocable: true\n---\nbody\n";
    let cmd = "---\nname: deploy-cmd\ndescription: COMMAND deploy.\n---\nbody\n";
    let (_tmp, paths) = stage_workspace(&[("deploy", skill, &[])], &[("deploy-cmd", cmd)]);
    let state = build_state(&paths);

    let info = invoke(
        state,
        meta_input("acme", "plug", "deploy*", EntryKind::Command),
    )
    .expect("kind-filtered glob resolves the single command candidate");

    assert_eq!(info.name, "deploy-cmd");
    assert!(matches!(info.kind, EntryKind::Command));
    assert_eq!(info.description.as_deref(), Some("COMMAND deploy."));
}

#[test]
fn exact_name_unaffected_by_wildcard_path() {
    let body = "---\nname: exact-one\ndescription: exact.\n---\nbody\n";
    let (_tmp, paths) = stage_workspace(&[("exact-one", body, &[])], &[]);
    let state = build_state(&paths);

    let info = invoke(
        state,
        meta_input("acme", "plug", "exact-one", EntryKind::Skill),
    )
    .expect("exact name resolves");

    assert_eq!(info.name, "exact-one");
    assert_eq!(info.description.as_deref(), Some("exact."));
    assert!(matches!(info.kind, EntryKind::Skill));
}

#[allow(dead_code)]
fn _path_buf_marker(_: PathBuf) {}
