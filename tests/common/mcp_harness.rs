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
use tome::mcp::tools::{get_skill, meta, search_skills};
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
    embedding_dim: Some(384),
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
    embedding_dim: None,
    files: &[],
    aux_urls: &[],
};

// ---------------------------------------------------------------------------
// Fixture staging.
//
// Factored from `tests/entry_e2e.rs::stage_workspace` (itself modelled on
// `tests/mcp_prompts.rs::stage_workspace_with`).
//
// FF3: this helper deliberately does NOT write `config.toml [catalogs]`.
// Catalog enrolment lives ONLY in the `workspace_catalogs` DB (seeded via
// `seed_catalog_enrolment`), which is the real `tome catalog add` shape.
// The previous `catalog::store::save` here was a masking dual-write that
// existed solely because `get_skill::handle` used to `store::load` the
// config; both MCP tools now resolve catalogs from the DB, so removing the
// write turns this whole staged corpus into an honest DB-only regression
// guard (a fresh-install `unknown_catalog` would now surface in tests).
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
            profile: None,
        },
    )
    .expect("open index db")
}

/// Write one plugin laid out on disk: the native `tome-plugin.toml` (Phase 8
/// cutover — the only manifest Tome reads) plus a legacy
/// `.claude-plugin/plugin.json` (so both-files coverage holds and convert
/// fixtures still have a CC manifest), `skills/<name>/SKILL.md` for each skill,
/// and `commands/<name>.md` for each command. `(name, body)` pairs carry the
/// full file body (frontmatter + content) verbatim.
pub fn write_plugin(
    catalog_root: &Path,
    plugin_name: &str,
    skills: &[(&str, &str)],
    commands: &[(&str, &str)],
) {
    let plugin_dir = catalog_root.join(plugin_name);
    std::fs::create_dir_all(&plugin_dir).unwrap();
    // Native manifest (Phase 8 cutover) — what `read_plugin_manifest` reads.
    std::fs::write(
        plugin_dir.join("tome-plugin.toml"),
        format!("name = \"{plugin_name}\"\nversion = \"1.0.0\"\n"),
    )
    .unwrap();
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
/// `lifecycle::enable`, and the catalog enrolment seeded into the
/// `workspace_catalogs` DB (NO `config.toml` — DB-only, FF3).
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
        // In-memory `Config` only — fed to `LifecycleDeps.config` below.
        // NOT persisted to disk (FF3): catalog enrolment is DB-only.
        let config = config_with_catalog("acme", &catalog_root);
        write_plugin(&catalog_root, "plug", skills, commands);

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
        // FF1: enrolment + cache symlink before enable — resolve_plugin_dir now
        // reads workspace_catalogs, not the in-memory Config.
        seed_catalog_enrolment(&paths, &catalog_root, "acme");
        lifecycle::enable(&id, &deps).expect("enable plugin");

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

    /// Build a [`McpHarness`] over this workspace whose embedder is the supplied
    /// one (Phase 12 / US2 — e.g. a `RemoteEmbedder` over a failing transport
    /// seam) and whose startup-frozen drift identity is `embedder_seed`. The
    /// `PromptRegistry` is built from the on-disk index exactly as `harness()`.
    pub fn harness_with_embedder(
        &self,
        embedder: Arc<dyn tome::embedding::Embedder>,
        embedder_seed: tome::index::MetaSeed,
    ) -> McpHarness {
        let registry = {
            let conn = open_index(&self.paths);
            let reg = PromptRegistry::build_for_workspace(
                &WorkspaceName::global(),
                &self.paths,
                &conn,
                false,
            )
            .expect("build prompt registry");
            drop(conn);
            reg
        };
        McpHarness::with_embedder(&self.paths, registry, None, None, embedder, embedder_seed)
    }
}

/// Build a `search_skills::Input` with defaults (top_k 10, no filters). Shared
/// by US2 tests that only vary the query text.
pub fn search_input(query: &str) -> search_skills::Input {
    search_skills::Input {
        query: query.into(),
        top_k: Some(10),
        catalog: None,
        plugin: None,
        description_max_chars: Some(150),
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
    /// test). Host harness is `None` (the `meta` tool then fails closed).
    pub fn with_registry(paths: &Paths, registry: PromptRegistry) -> Self {
        Self::with_host(paths, registry, None, None)
    }

    /// Build the harness with a caller-supplied `PromptRegistry`, an explicit
    /// `host_harness` (Phase 9 / US3 — the `meta` tool resolves its install
    /// target from it; `None` fails closed), and an optional `project_root` on
    /// the resolved scope (so the `meta` tool's PROJECT scope lands under a
    /// controlled dir rather than the launch CWD).
    pub fn with_host(
        paths: &Paths,
        registry: PromptRegistry,
        host_harness: Option<String>,
        project_root: Option<std::path::PathBuf>,
    ) -> Self {
        Self::with_embedder(
            paths,
            registry,
            host_harness,
            project_root,
            Arc::new(StubEmbedder::new()),
            tome::index::MetaSeed {
                name: STUB_EMBEDDER_ENTRY.name.into(),
                version: STUB_EMBEDDER_ENTRY.version.into(),
            },
        )
    }

    /// Build the harness with a caller-supplied embedder + the drift identity
    /// (`embedder_seed`) the on-disk index was stamped with. Phase 12 / US2: a
    /// `RemoteEmbedder` over a failing transport seam + the remote seed proves
    /// the MCP `search_skills` path fails CLOSED on a bad remote embedding (a
    /// clear tool error, never a degenerate KNN). The `embedder_entry` stays the
    /// stub registry entry (it only feeds the `embedder_model_id` telemetry
    /// field — a `&'static`; the DRIFT comparison uses `embedder_seed`).
    pub fn with_embedder(
        paths: &Paths,
        registry: PromptRegistry,
        host_harness: Option<String>,
        project_root: Option<std::path::PathBuf>,
        embedder: Arc<dyn tome::embedding::Embedder>,
        embedder_seed: tome::index::MetaSeed,
    ) -> Self {
        let reranker: Arc<dyn Reranker> = Arc::new(StubReranker::new());
        let mut scope = ResolvedScope::global_fallback();
        scope.project_root = project_root;
        let state = Arc::new(McpState {
            embedder,
            reranker: OnceCell::new_with(Some(reranker)),
            scope,
            paths: paths.clone(),
            // `embedder_entry` only feeds the `embedder_model_id` telemetry
            // field; the drift comparison uses `embedder_seed`.
            embedder_entry: &STUB_EMBEDDER_ENTRY,
            embedder_seed,
            reranker_entry: &STUB_RERANKER_ENTRY,
            prompt_registry: Arc::new(std::sync::RwLock::new(Arc::new(registry))),
            host_harness,
            last_search_ranks: std::sync::Mutex::new(std::collections::HashMap::new()),
            flush_signal: std::sync::Arc::new(tokio::sync::Notify::new()),
            enqueued_since_flush: std::sync::atomic::AtomicUsize::new(0),
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

    /// `tools/list` — returns the tool list the live server advertises,
    /// with the live `search_skills` description override applied. Mirrors
    /// [`Self::prompts_list`]: the real `ServerHandler::list_tools`
    /// requires a `RequestContext<RoleServer>` only obtainable over a live
    /// transport (see the module header), so the harness drives the same
    /// `Server::tools_listing` the production handler delegates to.
    pub fn tools_list(&self) -> Vec<rmcp::model::Tool> {
        self.server.tools_listing()
    }

    /// Seed the live `search_skills` description override on the server —
    /// the same seam `mcp::run` uses after construction
    /// ([`Server::override_search_skills_description`]). Lets a test drive
    /// the `tools/list` injection branch.
    pub fn override_search_skills_description(&mut self, description: impl Into<String>) {
        self.server.override_search_skills_description(description);
    }

    /// `prompts/get` — drive the live `prompts/get` route end-to-end via
    /// `prompts::handle_get` (the exact fn each route closure forwards
    /// to). `Ok` carries the rendered response; `Err` carries the
    /// `McpError` envelope the route would return on the wire.
    ///
    /// Holds [`CONTEXT_SEAM_MUTEX`] for the call so it can never overlap a
    /// render driven by [`Self::prompts_get_forcing_context_failure`]
    /// (which flips a process-global builder-failure flag). Cargo runs the
    /// tests in this binary in parallel threads; without this lock a
    /// sibling test's render could observe that flag mid-build.
    pub fn prompts_get(
        &self,
        name: &str,
        arguments: Option<Map<String, Value>>,
    ) -> Result<PromptGetResponse, McpError> {
        let _seam = lock_context_seam();
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

    /// Drive `prompts/get` with the context-builder seam tripped for the
    /// duration of the call (FR-012 / GAP-1, exit 28). Set the
    /// process-global [`FORCE_CONTEXT_BUILD_FAILURE`] flag, render, then
    /// clear it — all while holding [`CONTEXT_SEAM_MUTEX`], so the flag is
    /// never observable by a concurrent sibling render. Returns the
    /// `McpError` the live route surfaces (expected `substitution_failed`
    /// → exit 28).
    ///
    /// Pairing set + render + clear inside the one lock all renders share
    /// is the race-free shape (an RAII guard that merely set/cleared the
    /// flag would still leak the flip across the gap between flip and the
    /// next render's lock acquisition).
    pub fn prompts_get_forcing_context_failure(
        &self,
        name: &str,
        arguments: Option<Map<String, Value>>,
    ) -> Result<PromptGetResponse, McpError> {
        let _seam = lock_context_seam();
        // RAII so a panic mid-render still clears the flag before the lock
        // is released to the next render.
        let _flag = ForceContextBuildFailureFlag::set();
        self.rt.block_on(prompts::handle_get(
            self.state(),
            name.to_owned(),
            arguments,
        ))
    }

    /// `tools/call get_skill` — drive the live `get_skill` tool end-to-end
    /// via `get_skill::handle` (the exact fn the `#[tool]` method
    /// delegates to). Serialised on [`CONTEXT_SEAM_MUTEX`] for the same
    /// reason as [`Self::prompts_get`] (its render path also builds a
    /// `SubstitutionContext`).
    pub fn call_get_skill(&self, input: get_skill::Input) -> Result<get_skill::Output, McpError> {
        let _seam = lock_context_seam();
        self.rt.block_on(get_skill::handle(self.state(), input))
    }

    /// `tools/call search_skills` — drive the live `search_skills` tool
    /// end-to-end via `search_skills::handle`. (search_skills never builds
    /// a substitution context, but the lock is cheap and keeps every
    /// render-bearing harness call uniformly serialised.)
    pub fn call_search_skills(
        &self,
        input: search_skills::Input,
    ) -> Result<search_skills::Output, McpError> {
        let _seam = lock_context_seam();
        self.rt.block_on(search_skills::handle(self.state(), input))
    }

    /// `tools/call meta` — drive the live `meta` tool end-to-end via
    /// `meta::handle` (Phase 9 / US3).
    pub fn call_meta(&self, input: meta::Input) -> Result<meta::Output, McpError> {
        let _seam = lock_context_seam();
        self.rt.block_on(meta::handle(self.state(), input))
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
        // Phase 9 / US3 — the `meta` tool's slugs (1:1 with the CLI exit codes).
        "meta_skill_not_found" => TomeError::MetaSkillNotFound {
            id: "x".into(),
            available: "x".into(),
        },
        "meta_install_failed" => TomeError::MetaInstallFailed {
            skill_id: "x".into(),
            dir: PathBuf::from("/x"),
            source: std::io::Error::other("x"),
        },
        "no_harness_detected" => TomeError::NoHarnessDetected,
        // Phase 12 / US2 — a remote embedding failed content validation.
        "remote_embedding_invalid" => TomeError::RemoteEmbeddingInvalid { detail: "x".into() },
        // `search_skills` maps embedder drift to a custom `embedder_drift` code
        // (NOT `category().as_str()`), so this slug does not match a single
        // canonical category. Both name + version drift surface it; return the
        // shared drift exit code (41) and SKIP the category cross-check below by
        // returning early.
        "embedder_drift" => {
            return TomeError::EmbedderNameDrift {
                stored: "x".into(),
                configured: "x".into(),
            }
            .exit_code();
        }
        other => panic!(
            "unrecognised MCP error slug `{other}` — extend mcp_error_exit_code \
             when wiring a new error class through the in-process harness",
        ),
    };
    // Cross-check the reconstructed variant's category matches the slug
    // we observed, so the bridge can't silently drift from the source.
    assert_eq!(
        canonical.category().as_str(),
        slug,
        "harness slug→variant bridge drift: slug `{slug}` reconstructed as \
         category `{}`",
        canonical.category(),
    );
    canonical.exit_code()
}

// ---------------------------------------------------------------------------
// Render serialisation + the code-28 builder-failure seam (FR-012 / GAP-1).
//
// Cargo runs the tests in a single test binary across parallel threads.
// The `prompts/get` + `get_skill` render paths build a
// `SubstitutionContext` via the production `build_context_for_entry`,
// which consults the process-global
// `tome::mcp::substitution_helpers::FORCE_CONTEXT_BUILD_FAILURE` flag.
// The exit-28 test must flip that flag for ONE render — but a concurrent
// sibling render would otherwise observe the flip and fail spuriously.
//
// `CONTEXT_SEAM_MUTEX` serialises every render-bearing harness call (this
// is the `HOME_MUTEX` / `OVERRIDE_MUTEX` idiom from `tests/common/mod.rs`
// + `tests/harness_sync_stub.rs`). The exit-28 path sets the flag,
// renders, and clears it — all WITHIN the held lock — so the flip is
// invisible to every other render, which serialises behind the same lock.
// ---------------------------------------------------------------------------

use std::sync::{Mutex, MutexGuard, OnceLock};

/// Process-global serialisation lock for render-bearing harness calls.
static CONTEXT_SEAM_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

/// Acquire [`CONTEXT_SEAM_MUTEX`], recovering from a poisoned mutex (a
/// panic in one render must not cascade into the next render's setup).
fn lock_context_seam() -> MutexGuard<'static, ()> {
    CONTEXT_SEAM_MUTEX
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// RAII flag-flip for
/// [`tome::mcp::substitution_helpers::FORCE_CONTEXT_BUILD_FAILURE`]:
/// `true` on [`Self::set`], `false` on drop (surviving panics). Private —
/// only [`McpHarness::prompts_get_forcing_context_failure`] uses it, and
/// only while holding [`CONTEXT_SEAM_MUTEX`], so the flip never races a
/// concurrent render.
struct ForceContextBuildFailureFlag;

impl ForceContextBuildFailureFlag {
    fn set() -> Self {
        tome::mcp::substitution_helpers::FORCE_CONTEXT_BUILD_FAILURE
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Self
    }
}

impl Drop for ForceContextBuildFailureFlag {
    fn drop(&mut self) {
        tome::mcp::substitution_helpers::FORCE_CONTEXT_BUILD_FAILURE
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }
}
