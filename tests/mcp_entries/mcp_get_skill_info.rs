//! Phase 5 / US4.a — `get_skill_info` MCP tool end-to-end at the library API.
//!
//! Drives the real handler against a staged workspace + indexed plugin
//! using the StubEmbedder (no ONNX models needed). Mirrors the
//! `mcp_prompts.rs` staging discipline: a single tempdir hosts the
//! `Paths` root, the catalog clone, and a symlink wired up so
//! `paths.cache_dir_for(url)` resolves into the same on-disk directory
//! that the lifecycle pipeline indexed.
//!
//! Covers `contracts/mcp-tools-p5.md` § `get_skill_info`:
//!
//! - Skill-kind entry returns full description + when_to_use + resources.
//! - Command-kind entry omits the `resources` key entirely (FR-083).
//! - Per-directory cap of 5 + `"and N more"` sentinel.
//! - Subdir cap: per-subdir, NOT just top-level.
//! - Default `kind` parameter selects `skill`.
//! - Same name across both kinds → `kind` disambiguator selects.
//! - #295: not-found surfaces `get_skill`'s three-code surface —
//!   `unknown_catalog` / `unknown_plugin` / `unknown_skill` — with the same
//!   codes + messages get_skill emits (no more collapsed `entry_not_found`).
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
use tome::mcp::tools::get_skill_info::{self, Input};
use tome::plugin::PluginId;
use tome::plugin::identity::EntryKind;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{ResolvedScope, WorkspaceName};

use crate::common::{
    config_with_catalog, fabricate_models, lifecycle_paths, stub_embedder_seed, stub_reranker_seed,
    stub_summariser_seed,
};

// ---------------------------------------------------------------------------
// Fixture helpers (cloned from `tests/mcp_prompts.rs` — promotion to
// `crate::common::` is deferred per the orchestrator's brief; the staging code is
// non-trivial and the symlink discipline is test-suite-specific).
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
/// Relative paths starting with `subdir/` create the directory automatically.
type SkillFile<'a> = (&'a str, &'a str);

/// Stage a workspace with one plugin enabled. `skills` and `commands`
/// each carry `(name, body, extra_files)` tuples — `extra_files` are
/// resources written alongside the entry body (relevant only for skills;
/// for commands the slot is honoured but the contract elides the
/// resources field entirely).
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

    // Write the plugin directory under the catalog.
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
    // FF1: enrolment + cache symlink before enable — resolve_plugin_dir now
    // reads workspace_catalogs, not the in-memory Config.
    seed_catalog_enrolment(&paths, &catalog_root, "acme");
    lifecycle::enable(&id, &deps).expect("enable plugin");
    (tmp, paths)
}

/// Insert a `workspace_catalogs` row for `global` and symlink the
/// hashed cache dir to the on-disk catalog directory so
/// `paths.cache_dir_for(url)` resolves into a real layout. Lifted from
/// `tests/mcp_prompts.rs` — same discipline, same caveat about Unix vs
/// Windows (Windows uses a recursive copy fallback).
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

/// Build the `Arc<McpState>` the handler expects. The prompt registry is
/// empty — `get_skill_info` doesn't consume it; we still wire it so the
/// state shape stays valid.
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

/// Single-thread runtime + `block_on` — same shape as the prompts
/// integration tests.
fn invoke(
    state: Arc<McpState>,
    input: Input,
) -> Result<get_skill_info::SkillInfo, rmcp::ErrorData> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(get_skill_info::handle(state, input))
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
        Input {
            catalog: "acme".into(),
            plugin: "plug".into(),
            name: "with-resources".into(),
            kind: EntryKind::Skill,
        },
    )
    .expect("get_skill_info ok");

    assert_eq!(info.catalog, "acme");
    assert_eq!(info.plugin, "plug");
    assert_eq!(info.name, "with-resources");
    assert!(matches!(info.kind, EntryKind::Skill));
    assert_eq!(info.description, "A skill that ships sibling files.");
    assert_eq!(
        info.when_to_use.as_deref(),
        Some("When the user mentions resource enumeration."),
    );
    assert_eq!(info.plugin_version, "1.0.0");
    assert!(!info.user_invocable, "skills default user_invocable=false");
    assert!(info.path.ends_with("SKILL.md"));

    let res = info.resources.expect("skill MUST carry resources field");
    // Top-level files: only `config.json` (SKILL.md is excluded; the two
    // subdirs are in `directories`, not `files`).
    assert_eq!(res.files.len(), 1);
    assert!(res.files[0].ends_with("config.json"));

    // Subdirs alphabetised: examples, scripts.
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
        Input {
            catalog: "acme".into(),
            plugin: "plug".into(),
            name: "fix-issue".into(),
            kind: EntryKind::Command,
        },
    )
    .expect("get_skill_info ok for command");

    assert_eq!(info.name, "fix-issue");
    assert!(matches!(info.kind, EntryKind::Command));
    assert_eq!(info.description, "Fix a GitHub issue.");
    assert!(info.when_to_use.is_none());
    assert!(
        info.user_invocable,
        "commands default user_invocable=true per resolved-defaults table"
    );
    assert!(
        info.resources.is_none(),
        "FR-083: command-kind MUST omit the resources field; got Some(...)"
    );

    // Serialise + assert the JSON shape physically lacks the `resources` key.
    let json = serde_json::to_value(&info).expect("serialise");
    let obj = json.as_object().expect("object");
    assert!(
        !obj.contains_key("resources"),
        "command JSON must not include `resources` key, got: {}",
        json
    );
}

#[test]
fn heavy_directory_capped_with_sentinel() {
    // 7 top-level files → 5 returned + "and 2 more".
    // 7 files in subdir → same cap on the subdir.
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

    let info = invoke(
        state,
        Input {
            catalog: "acme".into(),
            plugin: "plug".into(),
            name: "heavy".into(),
            kind: EntryKind::Skill,
        },
    )
    .expect("ok");

    let res = info.resources.expect("skill resources");

    // Top-level: PER_DIRECTORY_CAP = 5 entries + 1 sentinel.
    assert_eq!(res.files.len(), 6, "5 files + 1 sentinel");
    assert_eq!(res.files[5], "and 2 more");

    // Subdir: same cap.
    let scripts = res.directories.get("scripts").unwrap();
    assert_eq!(scripts.len(), 6);
    assert_eq!(scripts[5], "and 2 more");
}

#[test]
fn default_kind_is_skill() {
    let skill_body = "---\nname: shared-name\ndescription: a skill.\n---\nbody\n";
    let (_tmp, paths) = stage_workspace(&[("shared-name", skill_body, &[])], &[]);
    let state = build_state(&paths);

    // Construct a JSON request without `kind` and round-trip via the
    // tool's deserialiser — proves the `default = skill` annotation
    // resolves correctly through the wire schema.
    let raw = serde_json::json!({
        "catalog": "acme",
        "plugin": "plug",
        "name": "shared-name",
    });
    let input: Input = serde_json::from_value(raw).expect("deserialise default kind");
    assert!(
        matches!(input.kind, EntryKind::Skill),
        "default kind must be Skill per FR-084",
    );

    let info = invoke(state, input).expect("ok");
    assert!(matches!(info.kind, EntryKind::Skill));
    assert_eq!(info.description, "a skill.");
}

#[test]
fn kind_disambiguates_same_name_across_kinds() {
    // Same NAME, both kinds enabled. The skill body uses
    // `user-invocable: true` so the prompts surface would surface both;
    // get_skill_info should pick the row matching the supplied `kind`.
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
        Input {
            catalog: "acme".into(),
            plugin: "plug".into(),
            name: "deploy".into(),
            kind: EntryKind::Skill,
        },
    )
    .expect("skill lookup ok");
    assert_eq!(skill_info.description, "SKILL deploy.");
    assert!(matches!(skill_info.kind, EntryKind::Skill));

    let cmd_info = invoke(
        state,
        Input {
            catalog: "acme".into(),
            plugin: "plug".into(),
            name: "deploy".into(),
            kind: EntryKind::Command,
        },
    )
    .expect("command lookup ok");
    assert_eq!(cmd_info.description, "COMMAND deploy.");
    assert!(matches!(cmd_info.kind, EntryKind::Command));
    assert!(cmd_info.resources.is_none());
}

#[test]
fn unknown_entry_surfaces_unknown_skill() {
    // #295: a real catalog + real plugin but an entry name that doesn't exist
    // now surfaces `unknown_skill` (matching `get_skill`), not the pre-#295
    // collapsed `entry_not_found`.
    let body = "---\nname: real\ndescription: ok.\n---\nbody\n";
    let (_tmp, paths) = stage_workspace(&[("real", body, &[])], &[]);
    let state = build_state(&paths);

    let err = invoke(
        state,
        Input {
            catalog: "acme".into(),
            plugin: "plug".into(),
            name: "does-not-exist".into(),
            kind: EntryKind::Skill,
        },
    )
    .expect_err("unknown entry must reject");

    let data = err.data.expect("structured error data");
    assert_eq!(
        data.get("code").and_then(|c| c.as_str()),
        Some("unknown_skill"),
        "expected unknown_skill code (matching get_skill), got: {}",
        data,
    );
    // The `unknown_skill` envelope carries catalog + plugin + name (byte-
    // identical to `get_skill`'s), NOT the pre-#295 `kind` field.
    assert_eq!(data.get("catalog").and_then(|c| c.as_str()), Some("acme"));
    assert_eq!(data.get("plugin").and_then(|c| c.as_str()), Some("plug"));
    assert_eq!(
        data.get("name").and_then(|c| c.as_str()),
        Some("does-not-exist"),
    );
    // The message matches get_skill's exact wording for the same case.
    assert_eq!(
        err.message,
        "skill `acme/plug/does-not-exist` is not enabled in the resolved scope",
    );
}

#[test]
fn unknown_catalog_surfaces_unknown_catalog() {
    // #295: get_skill_info now splits not-found the same way get_skill does —
    // an unenrolled catalog surfaces `unknown_catalog`, not `entry_not_found`.
    let body = "---\nname: real\ndescription: ok.\n---\nbody\n";
    let (_tmp, paths) = stage_workspace(&[("real", body, &[])], &[]);
    let state = build_state(&paths);

    let err = invoke(
        state,
        Input {
            catalog: "nonexistent".into(),
            plugin: "plug".into(),
            name: "real".into(),
            kind: EntryKind::Skill,
        },
    )
    .expect_err("unknown catalog must reject");

    let data = err.data.expect("structured error data");
    assert_eq!(
        data.get("code").and_then(|c| c.as_str()),
        Some("unknown_catalog"),
        "expected unknown_catalog code (matching get_skill), got: {data}",
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
    // #295: a real, enrolled catalog but a plugin with zero rows surfaces
    // `unknown_plugin` (matching get_skill's split), not the collapsed
    // `entry_not_found`.
    let body = "---\nname: real\ndescription: ok.\n---\nbody\n";
    let (_tmp, paths) = stage_workspace(&[("real", body, &[])], &[]);
    let state = build_state(&paths);

    let err = invoke(
        state,
        Input {
            catalog: "acme".into(),
            plugin: "no-such-plugin".into(),
            name: "real".into(),
            kind: EntryKind::Skill,
        },
    )
    .expect_err("unknown plugin must reject");

    let data = err.data.expect("structured error data");
    assert_eq!(
        data.get("code").and_then(|c| c.as_str()),
        Some("unknown_plugin"),
        "expected unknown_plugin code (matching get_skill), got: {data}",
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

    let err = invoke(
        state,
        Input {
            catalog: "".into(),
            plugin: "plug".into(),
            name: "real".into(),
            kind: EntryKind::Skill,
        },
    )
    .expect_err("empty catalog must reject");
    assert!(
        err.message.contains("non-empty"),
        "expected empty-field rejection, got: {}",
        err.message,
    );
}

#[test]
fn alphabetical_ordering_independent_of_creation_order() {
    // Write files in reverse-alphabetical order; the handler must still
    // return them alphabetised by basename.
    let extras: Vec<SkillFile<'_>> =
        vec![("zebra.txt", "z"), ("mango.txt", "m"), ("apple.txt", "a")];
    let body = "---\nname: ordered\ndescription: ordering check.\n---\nbody\n";
    let (_tmp, paths) = stage_workspace(&[("ordered", body, &extras)], &[]);
    let state = build_state(&paths);

    let info = invoke(
        state,
        Input {
            catalog: "acme".into(),
            plugin: "plug".into(),
            name: "ordered".into(),
            kind: EntryKind::Skill,
        },
    )
    .expect("ok");

    let res = info.resources.expect("resources present");
    assert_eq!(res.files.len(), 3);
    assert!(res.files[0].ends_with("apple.txt"));
    assert!(res.files[1].ends_with("mango.txt"));
    assert!(res.files[2].ends_with("zebra.txt"));
}

// Avoid an unused-import warning on platforms where some paths above
// don't reference `PathBuf` directly (the alias keeps the typed staging
// signature readable even when the test body never names the type).
#[allow(dead_code)]
fn _path_buf_marker(_: PathBuf) {}
