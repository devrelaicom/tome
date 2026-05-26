//! Phase 5 / US2.c — substitution pipeline end-to-end tests.
//!
//! Pins the **stage-ordering invariant** (Stage 1 built-ins → Stage 2
//! env passthrough), the **idempotence** of [`substitution::render`]
//! (NFR-007), and the integration through the two MCP surfaces that
//! invoke the pipeline at request time:
//!
//! - `mcp::prompts::handle_get` (the `prompts/get` capability — was
//!   wired in US1.c).
//! - `mcp::tools::get_skill::handle` (the read-side MCP tool — wired in
//!   US2.c).
//!
//! Both surfaces must render the same body identically so harnesses
//! see consistent values regardless of whether they call the
//! `prompts/get` MCP method or the `get_skill` tool. (The contract
//! distinguishes the two by purpose — prompts surface user-invocable
//! entries; `get_skill` is the read-side detail view — but the
//! substitution pipeline is shared.)
//!
//! Env-var serialisation: tests mutating the host environment via
//! `std::env::set_var` / `remove_var` serialise via a file-local
//! `ENV_MUTEX`, mirroring the convention in `tests/substitution_env.rs`
//! (Phase 4 / US3.c-1 `tests/harness_sync_stub.rs` `OVERRIDE_MUTEX`
//! pattern carried forward). `EnvVarGuard` is RAII — snapshot on
//! install, restore on drop — so panics inside a test don't leak
//! state to siblings.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use tempfile::TempDir;
use tokio::sync::OnceCell;
use tome::config::Config;
use tome::embedding::Reranker;
use tome::embedding::registry::lookup;
use tome::embedding::stub::{StubEmbedder, StubReranker};
use tome::index::{self, OpenOptions};
use tome::mcp::prompts::{self, PromptRegistry};
use tome::mcp::state::McpState;
use tome::mcp::tools::get_skill;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::substitution::{self, SubstitutionContext, SubstitutionContextBuilder};
use tome::workspace::{ResolvedScope, WorkspaceName};

use common::{
    PluginDataDirGuard, WorkspaceDataDirGuard, config_with_catalog, fabricate_models,
    lifecycle_paths, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed,
};

// --- Env serialisation discipline ----------------------------------------

static ENV_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

fn lock_env() -> MutexGuard<'static, ()> {
    ENV_MUTEX
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// RAII env-var guard mirroring `tests/substitution_env.rs`. Snapshot
/// previous value on install; restore on drop. Caller MUST hold
/// `ENV_MUTEX` for the lifetime of the guard.
struct EnvVarGuard {
    key: String,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: caller holds ENV_MUTEX; no other test mutates env.
        unsafe {
            std::env::set_var(key, value);
        }
        Self {
            key: key.to_owned(),
            previous,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        // SAFETY: ENV_MUTEX is still held by the test for the lifetime
        // of this guard.
        unsafe {
            match &self.previous {
                Some(v) => std::env::set_var(&self.key, v),
                None => std::env::remove_var(&self.key),
            }
        }
    }
}

// --- Library-level context plumbing --------------------------------------

fn ctx_builder(home: &Path) -> SubstitutionContextBuilder {
    let paths = lifecycle_paths(home);
    SubstitutionContext::builder()
        .catalog_name("test-catalog")
        .plugin_name("test-plugin")
        .plugin_version("1.2.3")
        .entry_name("hello")
        .entry_path(PathBuf::from("/plugins/x/skills/hello/SKILL.md"))
        .entry_dir(PathBuf::from("/plugins/x/skills/hello"))
        .plugin_root_dir(PathBuf::from("/plugins/x"))
        .workspace_name("global")
        .clock(time::OffsetDateTime::UNIX_EPOCH)
        .paths(paths)
}

fn ctx(home: &Path) -> SubstitutionContext {
    ctx_builder(home).build().expect("builder")
}

// --- Stage-ordering invariant -------------------------------------------

#[test]
fn stage_ordering_builtins_then_env_both_substitute() {
    // A single body referencing one Stage-1 built-in (`TOME_SKILL_NAME`)
    // and one Stage-2 env passthrough (`TOME_ENV_T225_STAGE_ORDER`)
    // must produce a body where BOTH references are resolved to their
    // respective values. The two stages compose in `substitution::render`
    // (built-ins first, env second).
    let _lock = lock_env();
    let _guard = EnvVarGuard::set("TOME_ENV_T225_STAGE_ORDER", "env-value");
    let tmp = tempfile::tempdir().unwrap();
    let _p = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));

    let body = "${TOME_SKILL_NAME} and ${TOME_ENV_T225_STAGE_ORDER}";
    let out = substitution::render(body, &ctx(tmp.path())).unwrap();
    assert_eq!(out, "hello and env-value");
}

// --- Idempotence (NFR-007) ----------------------------------------------

#[test]
fn render_called_twice_produces_identical_output() {
    // `substitution::render` is a pure function: built-ins read context
    // fields, env reads `std::env::var`. With a fixed context + a
    // stable host env, two consecutive calls must yield byte-identical
    // output (NFR-007 — substituted values are not re-scanned, so the
    // pipeline is single-pass and deterministic).
    let _lock = lock_env();
    let _guard = EnvVarGuard::set("TOME_ENV_T225_IDEMPOTENT", "stable");
    let tmp = tempfile::tempdir().unwrap();
    let _p = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));

    let body = "n=${TOME_SKILL_NAME} e=${TOME_ENV_T225_IDEMPOTENT} v=${TOME_PLUGIN_VERSION}";
    let first = substitution::render(body, &ctx(tmp.path())).unwrap();
    let second = substitution::render(body, &ctx(tmp.path())).unwrap();
    assert_eq!(first, second);
    assert_eq!(first, "n=hello e=stable v=1.2.3");
}

// --- End-to-end via mcp::prompts::handle_get ------------------------------

#[test]
fn mcp_prompts_get_runs_builtins_and_env() {
    // Stage a real workspace with one user-invocable command whose body
    // references both a Stage-1 built-in (`TOME_PLUGIN_NAME`) and a
    // Stage-2 env passthrough (`TOME_ENV_T225_PROMPTS_GET`). Drive
    // `mcp::prompts::handle_get` and assert both references are
    // resolved.
    let _lock = lock_env();
    let _guard = EnvVarGuard::set("TOME_ENV_T225_PROMPTS_GET", "prompts-env");
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    let _p = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));

    let cmd_body = "---\nname: pipeline\ndescription: pipeline test.\n---\nplugin=${TOME_PLUGIN_NAME} env=${TOME_ENV_T225_PROMPTS_GET}\n";
    stage_workspace(&tmp, &paths, &[], &[("pipeline", cmd_body)]);

    let state = build_state_for_prompts(&paths);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let response = rt
        .block_on(prompts::handle_get(state, "plug__pipeline".into(), None))
        .expect("prompts/get ok");

    assert_eq!(response.messages.len(), 1);
    let text = match &response.messages[0].content {
        rmcp::model::PromptMessageContent::Text { text } => text.clone(),
        other => panic!("expected text content, got {other:?}"),
    };
    // Built-in (`plug`) and env (`prompts-env`) BOTH resolved.
    assert!(
        text.contains("plugin=plug"),
        "Stage 1 built-in not substituted in prompts/get output: {text:?}",
    );
    assert!(
        text.contains("env=prompts-env"),
        "Stage 2 env passthrough not substituted in prompts/get output: {text:?}",
    );
    // No leftover ${...} brackets for either reference.
    assert!(
        !text.contains("${TOME_PLUGIN_NAME}"),
        "Stage 1 reference left verbatim: {text:?}",
    );
    assert!(
        !text.contains("${TOME_ENV_T225_PROMPTS_GET}"),
        "Stage 2 reference left verbatim: {text:?}",
    );
}

// --- End-to-end via mcp::tools::get_skill::handle ------------------------

#[test]
fn mcp_get_skill_runs_builtins_and_env() {
    // Stage a real workspace with one skill (NOT user-invocable — but
    // `get_skill` reads any enabled skill regardless of the prompts
    // gate). Drive the `get_skill` MCP tool and assert the rendered
    // body has both Stage-1 + Stage-2 references resolved.
    let _lock = lock_env();
    let _guard = EnvVarGuard::set("TOME_ENV_T225_GET_SKILL", "skill-env");
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    let _p = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));

    let skill_body = "---\nname: pipe-skill\ndescription: pipe test.\n---\ncat=${TOME_CATALOG_NAME} env=${TOME_ENV_T225_GET_SKILL}\n";
    stage_workspace(&tmp, &paths, &[("pipe-skill", skill_body)], &[]);

    let state = build_state_for_get_skill(&paths);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let output = rt
        .block_on(get_skill::handle(
            state,
            get_skill::Input {
                catalog: "acme".to_string(),
                plugin: "plug".to_string(),
                name: "pipe-skill".to_string(),
            },
        ))
        .expect("get_skill ok");

    let text = output.content;
    assert!(
        text.contains("cat=acme"),
        "Stage 1 built-in not substituted in get_skill output: {text:?}",
    );
    assert!(
        text.contains("env=skill-env"),
        "Stage 2 env passthrough not substituted in get_skill output: {text:?}",
    );
    assert!(
        !text.contains("${TOME_CATALOG_NAME}"),
        "Stage 1 reference left verbatim: {text:?}",
    );
    assert!(
        !text.contains("${TOME_ENV_T225_GET_SKILL}"),
        "Stage 2 reference left verbatim: {text:?}",
    );
}

// --- Workspace staging helpers -------------------------------------------
//
// Mirrors `tests/mcp_prompts.rs::stage_workspace_with` minus the
// implicit `paths` allocation — callers pre-build `paths` so the same
// instance flows into both `lifecycle::enable` and the MCP state.

fn stage_workspace(
    tmp: &TempDir,
    paths: &tome::paths::Paths,
    skills: &[(&str, &str)],
    commands: &[(&str, &str)],
) {
    fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(paths);

    let catalog_root = tmp.path().join("catalog");
    fs::create_dir_all(&catalog_root).unwrap();
    let config = config_with_catalog("acme", &catalog_root);
    write_plugin(&catalog_root, "plug", skills, commands);

    // Persist the config so `mcp::tools::get_skill::handle` (which loads
    // `store::load(&paths.global_config_file)` to validate the catalog
    // name) sees the catalog enrolment.
    save_config(paths, &config);

    let embedder = StubEmbedder::new();
    let scope = tome::workspace::Scope(WorkspaceName::global());
    let deps = LifecycleDeps {
        paths,
        scope: &scope,
        config: &config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };
    let id: PluginId = "acme/plug".parse().unwrap();
    lifecycle::enable(&id, &deps).expect("enable plugin");

    seed_catalog_enrolment(paths, &catalog_root, "acme");
}

fn save_config(paths: &tome::paths::Paths, config: &Config) {
    if let Some(parent) = paths.global_config_file.parent() {
        fs::create_dir_all(parent).expect("create config parent");
    }
    tome::catalog::store::save(&paths.global_config_file, config).expect("save config");
}

fn write_plugin(
    catalog_root: &Path,
    plugin_name: &str,
    skills: &[(&str, &str)],
    commands: &[(&str, &str)],
) {
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

fn build_state_for_prompts(paths: &tome::paths::Paths) -> Arc<McpState> {
    let conn = open_index(paths);
    let registry = PromptRegistry::build_for_workspace(&WorkspaceName::global(), paths, &conn)
        .expect("build prompt registry");
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

fn build_state_for_get_skill(paths: &tome::paths::Paths) -> Arc<McpState> {
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
        prompt_registry: Arc::new(PromptRegistry::default()),
    })
}
