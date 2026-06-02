//! In-process MCP test harness (Phase 7 / FR-012, closes CONCERNS GAP-1).
//!
//! Constructs and drives a REAL [`tome::mcp::server::Server`] instance
//! in-process via the library API — no spawned `tome mcp` binary, no
//! `rmcp` stdio handshake, no ONNX model (StubEmbedder only, so the
//! harness is fast + CI-safe). The harness can issue `prompts/list`,
//! `prompts/get`, and tool calls (`get_skill`, `search_skills`) against
//! the live server and observe the outcome OR the resulting `McpError`
//! envelope + the matching `TomeError` exit code.
//!
//! ## Why these entry points (and not a synthetic `RequestContext`)
//!
//! rmcp's `PromptContext` / `ToolCallContext` both carry a
//! `RequestContext<RoleServer>`, which in turn requires a live
//! `Peer<RoleServer>` — only obtainable from a running `rmcp` service
//! over a transport. Constructing one test-side is impractical, which is
//! exactly why `tests/mcp_prompts.rs` (header, "prompts/get tests")
//! drives the silent-compute entry points the live router wraps
//! identically:
//!
//! - `prompts/list`  → [`Server::prompt_router_ref().list_all()`] — what
//!   `ServerHandler::list_prompts` returns verbatim.
//! - `prompts/get`   → [`tome::mcp::prompts::handle_get`] — the exact fn
//!   each `PromptRoute`'s closure forwards to (`make_get_handler` →
//!   `get_prompt_future` → `handle_get`).
//! - `get_skill`     → [`get_skill::handle`] — the exact fn the
//!   `#[tool(name = "get_skill")]` method delegates to.
//! - `search_skills` → [`search_skills::handle`] — ditto.
//!
//! The harness owns a real `Server` (built via `Server::new`, which
//! constructs the real `PromptRouter` via `prompts::build_router`) so the
//! routes are wired exactly as production wires them; the drivers reach
//! the same handlers the live router dispatches to. This is end-to-end
//! "through the real server instance" without a transport.
//!
//! ## Async boundary
//!
//! The MCP handlers are `async`; the harness drives them on a
//! single-thread tokio runtime owned per [`McpHarness`]. This does NOT
//! violate `tests/sync_boundary.rs` — that test scans `src/`, never
//! `tests/`, and `src/mcp/` is the sanctioned async island regardless.

#![allow(dead_code)] // each test file uses a subset of these helpers

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rmcp::ErrorData as McpError;
use serde_json::{Map, Value};
use tempfile::TempDir;
use tokio::runtime::Runtime;
use tokio::sync::OnceCell;
use tome::embedding::Reranker;
use tome::embedding::registry::{ModelEntry, ModelKind};
use tome::embedding::stub::{StubEmbedder, StubReranker};
use tome::index::{self, OpenOptions};
use tome::mcp::prompts::{self, PromptDescriptor, PromptGetResponse, PromptRegistry};
use tome::mcp::server::Server;
use tome::mcp::state::McpState;
use tome::mcp::tools::{get_skill, search_skills};
use tome::paths::Paths;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{ResolvedScope, Scope, WorkspaceName};

use super::{
    config_with_catalog, fabricate_models, lifecycle_paths, stub_embedder_seed, stub_reranker_seed,
    stub_summariser_seed,
};

// ---------------------------------------------------------------------------
// Stub model registry entries.
//
// `search_skills::handle` reads `state.embedder_entry.{name,version}` for
// drift detection against the index `meta` row, which `lifecycle::enable`
// seeds with `stub_embedder_seed()`. Using the real `bge-*` registry
// entries (via `lookup`) would mismatch the stub-seeded index and trip
// `embedder_drift`. Mirrors `tests/entry_e2e.rs::{STUB_EMBEDDER_ENTRY,
// STUB_RERANKER_ENTRY}` — file-local copies kept in lockstep with the
// stub embedder/reranker's reported identity.
// ---------------------------------------------------------------------------

static STUB_EMBEDDER_ENTRY: ModelEntry = ModelEntry {
    name: "stub-embedder",
    version: "0",
    kind: ModelKind::Embedder,
    source_url: "stub://embedder",
    sha256: "0000000000000000000000000000000000000000000000000000000000000000",
    size_bytes: 0,
    licence: "MIT",
    files: &[],
    aux_urls: &[],
};

static STUB_RERANKER_ENTRY: ModelEntry = ModelEntry {
    name: "stub-reranker",
    version: "0",
    kind: ModelKind::Reranker,
    source_url: "stub://reranker",
    sha256: "0000000000000000000000000000000000000000000000000000000000000000",
    size_bytes: 0,
    licence: "MIT",
    files: &[],
    aux_urls: &[],
};

// ---------------------------------------------------------------------------
// Fixture staging.
//
// Factored from `tests/entry_e2e.rs::stage_workspace` (itself modelled on
// `tests/mcp_prompts.rs::stage_workspace_with`). The decisive difference
// from the `mcp_prompts.rs` variant is that this one PERSISTS
// `config.toml` to disk via `catalog::store::save`, which
// `get_skill::handle` requires (it calls `store::load(global_config_file)`
// and returns `unknown_catalog` otherwise).
//
// Promotion to a shared `tests/common/` helper (this module) is the
// "fifth consumer" fold the entry_e2e header anticipated: mcp_prompts,
// mcp_prompts_get_error_json_shape, entry_e2e, and now exit_codes_e2e_mcp
// all stage the same shape.
// ---------------------------------------------------------------------------

/// Open (and bootstrap if absent) the central index DB with the stub
/// identity seeds — matches what `lifecycle::enable` stamps.
pub fn open_index(paths: &Paths) -> rusqlite::Connection {
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

/// Write one plugin laid out on disk: `.claude-plugin/plugin.json`,
/// `skills/<name>/SKILL.md` for each skill, `commands/<name>.md` for each
/// command. `(name, body)` pairs carry the full file body (frontmatter +
/// content) verbatim.
pub fn write_plugin(
    catalog_root: &Path,
    plugin_name: &str,
    skills: &[(&str, &str)],
    commands: &[(&str, &str)],
) {
    let plugin_dir = catalog_root.join(plugin_name);
    std::fs::create_dir_all(plugin_dir.join(".claude-plugin")).unwrap();
    std::fs::write(
        plugin_dir.join(".claude-plugin").join("plugin.json"),
        format!(r#"{{"name": "{plugin_name}", "version": "1.0.0"}}"#),
    )
    .unwrap();
    for (name, body) in skills {
        let dir = plugin_dir.join("skills").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), body).unwrap();
    }
    if !commands.is_empty() {
        let cmd_dir = plugin_dir.join("commands");
        std::fs::create_dir_all(&cmd_dir).unwrap();
        for (name, body) in commands {
            std::fs::write(cmd_dir.join(format!("{name}.md")), body).unwrap();
        }
    }
}

/// Seed the central DB's `workspace_catalogs` enrolment for the
/// privileged `global` workspace + arrange the content-addressed cache
/// dir (`paths.cache_dir_for(url)`) to point at `catalog_root` so the
/// registry's `resolve_catalog_path` → `read_catalog_manifest` walk hits
/// the real on-disk fixture.
pub fn seed_catalog_enrolment(paths: &Paths, catalog_root: &Path, catalog_name: &str) {
    let url = format!("file://{}", catalog_root.display());
    let conn = open_index(paths);
    tome::index::workspace_catalogs::insert(&conn, "global", catalog_name, &url, "main")
        .expect("seed workspace_catalogs");
    drop(conn);

    let cache_dir = paths.cache_dir_for(&url);
    if let Some(parent) = cache_dir.parent() {
        std::fs::create_dir_all(parent).expect("create catalogs parent");
    }
    if !cache_dir.exists() {
        #[cfg(unix)]
        std::os::unix::fs::symlink(catalog_root, &cache_dir).expect("symlink catalog cache");
        #[cfg(not(unix))]
        {
            fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
                std::fs::create_dir_all(dst)?;
                for entry in std::fs::read_dir(src)? {
                    let entry = entry?;
                    let to = dst.join(entry.file_name());
                    if entry.file_type()?.is_dir() {
                        copy_dir(&entry.path(), &to)?;
                    } else {
                        std::fs::copy(entry.path(), &to)?;
                    }
                }
                Ok(())
            }
            copy_dir(catalog_root, &cache_dir).expect("copy catalog cache");
        }
    }
}

/// A staged workspace: one catalog (`acme`) holding one plugin (`plug`)
/// with the supplied skills + commands, enabled + indexed via
/// `lifecycle::enable`, `config.toml` persisted, and the catalog
/// enrolment seeded.
///
/// `tmp` MUST outlive the fixture (it owns the on-disk tree); the public
/// fields let tests reach the catalog root for post-enable mutations
/// (e.g. deleting a plugin dir to force `EntryNotFound`).
pub struct StagedWorkspace {
    pub tmp: TempDir,
    pub paths: Paths,
    pub catalog_root: PathBuf,
    pub catalog_name: String,
    pub plugin_name: String,
}

impl StagedWorkspace {
    /// Stage `acme/plug` with the supplied entries. Enables + indexes the
    /// plugin against the `global` workspace using the StubEmbedder.
    pub fn stage(skills: &[(&str, &str)], commands: &[(&str, &str)]) -> Self {
        let tmp = TempDir::new().unwrap();
        let paths = lifecycle_paths(tmp.path());
        std::fs::create_dir_all(&paths.root).unwrap();
        fabricate_models(&paths);

        let catalog_root = tmp.path().join("catalog");
        std::fs::create_dir_all(&catalog_root).unwrap();
        let config = config_with_catalog("acme", &catalog_root);
        write_plugin(&catalog_root, "plug", skills, commands);

        // Persist config.toml so `get_skill::handle` (disk-loaded config)
        // and `lifecycle::enable` (in-memory config) agree on the catalog.
        if let Some(parent) = paths.global_config_file.parent() {
            std::fs::create_dir_all(parent).expect("create config parent");
        }
        tome::catalog::store::save(&paths.global_config_file, &config).expect("save config");

        let embedder = StubEmbedder::new();
        let scope = Scope(WorkspaceName::global());
        let deps = LifecycleDeps {
            paths: &paths,
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

        seed_catalog_enrolment(&paths, &catalog_root, "acme");

        Self {
            tmp,
            paths,
            catalog_root,
            catalog_name: "acme".to_owned(),
            plugin_name: "plug".to_owned(),
        }
    }

    /// Absolute on-disk path of the staged plugin's directory inside the
    /// catalog fixture (`<catalog_root>/<plugin>`).
    pub fn plugin_dir(&self) -> PathBuf {
        self.catalog_root.join(&self.plugin_name)
    }

    /// Build a [`McpHarness`] (real `Server` + driver) over this staged
    /// workspace's resolved `global` scope.
    pub fn harness(&self) -> McpHarness {
        McpHarness::new(&self.paths)
    }
}

// ---------------------------------------------------------------------------
// The in-process server driver.
// ---------------------------------------------------------------------------

/// Owns a real [`Server`] built over the staged workspace's `McpState`
/// (StubEmbedder, StubReranker, resolved `global` scope, a
/// `PromptRegistry` built from the on-disk index) plus a single-thread
/// tokio runtime to drive the async handlers.
pub struct McpHarness {
    server: Server,
    state: Arc<McpState>,
    rt: Runtime,
}

impl McpHarness {
    /// Build the harness for the `global` workspace rooted at `paths`.
    /// The `PromptRegistry` is built from the on-disk index exactly as
    /// `mcp::run`'s `build_prompt_registry` does (personas off).
    pub fn new(paths: &Paths) -> Self {
        let registry = {
            let conn = open_index(paths);
            let reg =
                PromptRegistry::build_for_workspace(&WorkspaceName::global(), paths, &conn, false)
                    .expect("build prompt registry");
            drop(conn);
            reg
        };
        Self::with_registry(paths, registry)
    }

    /// Build the harness with a caller-supplied `PromptRegistry` (e.g.
    /// `PromptRegistry::default()` when only the tool surface is under
    /// test).
    pub fn with_registry(paths: &Paths, registry: PromptRegistry) -> Self {
        let reranker: Arc<dyn Reranker> = Arc::new(StubReranker::new());
        let state = Arc::new(McpState {
            embedder: Arc::new(StubEmbedder::new()),
            reranker: OnceCell::new_with(Some(reranker)),
            scope: ResolvedScope::global_fallback(),
            paths: paths.clone(),
            // Stub identity entries so search_skills' drift check agrees
            // with the stub-seeded index `meta`.
            embedder_entry: &STUB_EMBEDDER_ENTRY,
            reranker_entry: &STUB_RERANKER_ENTRY,
            prompt_registry: Arc::new(registry),
        });

        // Build the REAL server — `Server::new` constructs the real
        // `PromptRouter` (via `prompts::build_router`) + tool router, so
        // the routes are wired exactly as `mcp::run` wires them.
        let server = Server::new(state.clone());

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");

        Self { server, state, rt }
    }

    /// Read-only borrow of the live `Server` (for tests that want to
    /// introspect the router surfaces directly).
    pub fn server(&self) -> &Server {
        &self.server
    }

    /// Shared `McpState` the server was built over.
    pub fn state(&self) -> Arc<McpState> {
        self.state.clone()
    }

    /// `prompts/list` — returns the prompt descriptors the live server
    /// advertises, via the same `PromptRouter::list_all()` that
    /// `ServerHandler::list_prompts` returns verbatim.
    pub fn prompts_list(&self) -> Vec<PromptDescriptor> {
        self.server.prompt_router_ref().list_all()
    }

    /// The set of advertised prompt names (convenience over
    /// [`Self::prompts_list`]).
    pub fn prompt_names(&self) -> Vec<String> {
        self.prompts_list().into_iter().map(|p| p.name).collect()
    }

    /// `prompts/get` — drive the live `prompts/get` route end-to-end via
    /// `prompts::handle_get` (the exact fn each route closure forwards
    /// to). `Ok` carries the rendered response; `Err` carries the
    /// `McpError` envelope the route would return on the wire.
    pub fn prompts_get(
        &self,
        name: &str,
        arguments: Option<Map<String, Value>>,
    ) -> Result<PromptGetResponse, McpError> {
        self.rt.block_on(prompts::handle_get(
            self.state(),
            name.to_owned(),
            arguments,
        ))
    }

    /// Convenience: drive `prompts/get` and return the single rendered
    /// user-message text on success.
    pub fn prompts_get_text(
        &self,
        name: &str,
        arguments: Option<Map<String, Value>>,
    ) -> Result<String, McpError> {
        let resp = self.prompts_get(name, arguments)?;
        assert_eq!(resp.messages.len(), 1, "single user-role message");
        match &resp.messages[0].content {
            rmcp::model::PromptMessageContent::Text { text } => Ok(text.clone()),
            other => panic!("expected text content, got {other:?}"),
        }
    }

    /// `tools/call get_skill` — drive the live `get_skill` tool end-to-end
    /// via `get_skill::handle` (the exact fn the `#[tool]` method
    /// delegates to).
    pub fn call_get_skill(&self, input: get_skill::Input) -> Result<get_skill::Output, McpError> {
        self.rt.block_on(get_skill::handle(self.state(), input))
    }

    /// `tools/call search_skills` — drive the live `search_skills` tool
    /// end-to-end via `search_skills::handle`.
    pub fn call_search_skills(
        &self,
        input: search_skills::Input,
    ) -> Result<search_skills::Output, McpError> {
        self.rt.block_on(search_skills::handle(self.state(), input))
    }
}

// ---------------------------------------------------------------------------
// McpError → exit-code bridge.
//
// The MCP handlers return `McpError` (rmcp `ErrorData`), whose
// `data.code` slug is 1:1 with `TomeError::category()`. To assert
// `err.exit_code()` end-to-end, [`mcp_error_exit_code`] reconstructs the
// canonical `TomeError` for the observed slug and returns its real
// `.exit_code()` — so the assertion exercises the production
// `TomeError::exit_code()` mapping (not a hand-copied number), while the
// slug itself proves the live server reached that error class.
// ---------------------------------------------------------------------------

/// Extract the `data.code` slug from an `McpError` envelope.
pub fn mcp_error_slug(err: &McpError) -> String {
    err.data
        .as_ref()
        .and_then(|d| d.get("code"))
        .and_then(|c| c.as_str())
        .unwrap_or_else(|| panic!("McpError missing data.code slug: {err:?}"))
        .to_owned()
}

/// Map an `McpError`'s `data.code` slug to the exit code its underlying
/// [`tome::error::TomeError`] variant carries — routed THROUGH
/// `TomeError::exit_code()` so the test asserts the real mapping.
///
/// Panics on an unrecognised slug so a new MCP error slug (or a renamed
/// category) surfaces loudly rather than silently asserting `None`.
pub fn mcp_error_exit_code(err: &McpError) -> i32 {
    use tome::error::TomeError;
    let slug = mcp_error_slug(err);
    let canonical: TomeError = match slug.as_str() {
        "plugin_data_dir_write_failed" => TomeError::PluginDataDirWriteFailed {
            path: PathBuf::from("/x"),
            source: std::io::Error::other("x"),
        },
        "workspace_data_dir_write_failed" => TomeError::WorkspaceDataDirWriteFailed {
            path: PathBuf::from("/x"),
            source: std::io::Error::other("x"),
        },
        "prompt_argument_mismatch" => TomeError::PromptArgumentMismatch {
            expected: 0,
            supplied: 0,
        },
        "entry_not_found" => TomeError::EntryNotFound {
            catalog: "x".into(),
            plugin: "x".into(),
            name: "x".into(),
            kind: "x".into(),
        },
        "substitution_failed" => TomeError::SubstitutionFailed { reason: "x".into() },
        "invalid_argument_frontmatter" => TomeError::InvalidArgumentFrontmatter {
            file: PathBuf::from("/x"),
            reason: "x".into(),
        },
        "skill_frontmatter_parse_error" => TomeError::SkillFrontmatterParseError {
            file: PathBuf::from("/x"),
            message: "x".into(),
        },
        other => panic!(
            "unrecognised MCP error slug `{other}` — extend mcp_error_exit_code \
             when wiring a new error class through the in-process harness",
        ),
    };
    // Cross-check the reconstructed variant's category matches the slug
    // we observed, so the bridge can't silently drift from the source.
    assert_eq!(
        canonical.category(),
        slug,
        "harness slug→variant bridge drift: slug `{slug}` reconstructed as \
         category `{}`",
        canonical.category(),
    );
    canonical.exit_code()
}

// ---------------------------------------------------------------------------
// Code-28 builder-failure seam guard (FR-012 / GAP-1).
// ---------------------------------------------------------------------------

/// RAII guard flipping
/// [`tome::mcp::substitution_helpers::FORCE_CONTEXT_BUILD_FAILURE`] on,
/// restoring `false` on drop (surviving panics). While installed, the
/// `prompts/get` + `get_skill` render paths' `SubstitutionContext`
/// builder omits a required field, so `.build()` fails and the caller
/// wraps it as `TomeError::SubstitutionFailed` (exit 28) — the genuine
/// production wrap, otherwise unreachable through fixtures.
///
/// Single-threaded harness drives one render at a time, so the
/// process-global flag needs no extra serialisation here; tests that
/// drive renders concurrently against this slot would have to add their
/// own.
pub struct ForceContextBuildFailureGuard;

impl ForceContextBuildFailureGuard {
    pub fn install() -> Self {
        tome::mcp::substitution_helpers::FORCE_CONTEXT_BUILD_FAILURE
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Self
    }
}

impl Drop for ForceContextBuildFailureGuard {
    fn drop(&mut self) {
        tome::mcp::substitution_helpers::FORCE_CONTEXT_BUILD_FAILURE
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
}
