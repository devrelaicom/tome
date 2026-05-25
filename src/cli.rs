//! `clap` derive definitions. Globals (`--json`, `-v`/`-vv`, plus the
//! Phase 4 `--workspace <name>`) live on the top-level `Cli`; `--force`
//! is per-subcommand but keeps the same name everywhere (FR-021).
//! `--help` is auto-supplied by clap; `--version` is intercepted by a
//! pre-parse hook in `main.rs` so the output can include embedder +
//! reranker identities (FR-021a).
//!
//! Phase 4 / F10 deleted the `--global` flag. Workspace identity is a
//! validated [`crate::workspace::WorkspaceName`] checked against the
//! central registry; the privileged `global` workspace is the silent
//! default when no `--workspace` flag, `TOME_WORKSPACE` env var, or
//! project marker is found.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "tome",
    about,
    long_about = None,
    // `--version` is intercepted by a pre-parse hook in `main.rs` so the
    // output can include embedder + reranker identities and honour
    // `--json`. clap's auto handler can't do either, hence the override.
    disable_version_flag = true,
)]
pub struct Cli {
    /// Emit machine-readable JSON on stdout instead of human text.
    #[arg(long, global = true)]
    pub json: bool,

    /// Increase log verbosity. `-v` = info, `-vv` = debug. Env: TOME_LOG.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    #[command(flatten)]
    pub scope: GlobalScopeArgs,

    #[command(subcommand)]
    pub command: Command,
}

/// Workspace selection. Flattened into `Cli` so the flag appears at the
/// top level **and** on every subcommand (clap's `global = true`).
///
/// Phase 4 / F10 collapses the Phase 3 `--workspace <path>` /
/// `--global` pair into a single name-keyed `--workspace <name>` flag.
/// The privileged `global` workspace is the silent default — no flag
/// needed.
#[derive(Debug, Default, clap::Args)]
pub struct GlobalScopeArgs {
    /// Use the named workspace from the central registry. Must already
    /// exist; create via `tome workspace init <name>`. When omitted, the
    /// resolver consults `TOME_WORKSPACE` and the project-marker walk
    /// before falling back to the privileged `global` workspace.
    #[arg(long, global = true, value_name = "NAME")]
    pub workspace: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Manage registered catalogs.
    #[command(subcommand)]
    Catalog(CatalogCommand),
    /// Manage plugins from registered catalogs. Run with no subcommand to
    /// drop into the interactive catalog → plugin → action browse flow
    /// (FR-050; refuses on non-TTY per FR-051).
    Plugin(PluginArgs),
    /// Manage on-disk embedding / reranker model artefacts.
    #[command(subcommand)]
    Models(ModelsCommand),
    /// Search enabled skills across every catalog.
    Query(QueryArgs),
    /// Force re-embedding of one or many skills outside the
    /// `tome catalog update` schedule. Use for embedder upgrades or
    /// integrity recovery. See `contracts/reindex.md`.
    Reindex(ReindexArgs),
    /// Report the health of every Phase 2 subsystem (models, index, drift).
    /// Exit 0 when everything is healthy; exit 1 on degraded or unhealthy.
    Status(StatusArgs),
    /// Run as a stdio MCP server backed by the resolved scope's index.
    /// stdin / stdout carry the MCP protocol exclusively; diagnostic
    /// logs go to `${XDG_STATE_HOME}/tome/mcp.log`. Designed to be
    /// launched by an MCP-compliant harness (Claude Code, Codex, Cursor,
    /// Gemini CLI, OpenCode, …) as a child process. The global `--json`
    /// flag is intentionally ignored — the protocol IS the structured
    /// output.
    Mcp(McpArgs),
    /// Inspect or create per-project workspaces. See
    /// `contracts/workspace-info.md` and `contracts/workspace-init.md`.
    Workspace(WorkspaceArgs),
    /// Comprehensive diagnostic. Reports every subsystem (workspace,
    /// models, index, drift, catalog caches, harnesses), classifies
    /// overall health, and lists suggested fixes. With `--fix`,
    /// applies the three safe repair classes (re-download models,
    /// re-clone broken catalog caches, forward-migrate the schema).
    Doctor(DoctorArgs),
}

#[derive(Debug, clap::Args)]
pub struct DoctorArgs {
    /// Apply the three safe automatic repairs (re-download missing or
    /// corrupt models, re-clone broken catalog caches, forward-migrate
    /// the index schema). Destructive repairs are never automatic.
    #[arg(long)]
    pub fix: bool,
    /// Rehash each installed model's primary file against its pinned
    /// SHA-256. Slower than the default but catches silent on-disk
    /// corruption.
    #[arg(long)]
    pub verify: bool,
}

#[derive(Debug, clap::Args)]
pub struct WorkspaceArgs {
    #[command(subcommand)]
    pub command: WorkspaceCommand,
}

#[derive(Debug, Subcommand)]
pub enum WorkspaceCommand {
    /// Report the resolved workspace context for the current invocation.
    /// Read-only; honours `--workspace <name>` like every other command.
    /// Bootstrap-not-yet is informational, not an error.
    Info,
    /// Create a `.tome/` workspace at `<path>` (defaults to current
    /// directory). Atomic — a SIGINT or crash leaves either no `.tome/`
    /// or a complete one, never a partial.
    Init(WorkspaceInitArgs),
    /// Bind the current project directory to the named workspace.
    /// Creates / overwrites `<cwd>/.tome/config.toml` so subsequent
    /// Tome invocations from this tree resolve to `<name>` via the
    /// project-marker walk. The atomic-directory landing means a
    /// SIGINT mid-bind never leaves a partial `.tome/`. Phase 4 / US1.a
    /// stubs the harness-sync seam; US1.b wires the real sync.
    ///
    /// Note: the `<name>` argument always takes precedence; the global
    /// `--workspace` flag is ignored for this subcommand.
    Use(WorkspaceUseArgs),
}

#[derive(Debug, clap::Args)]
pub struct WorkspaceUseArgs {
    /// Workspace name (must already exist in the central registry; create
    /// via `tome workspace add` — US2).
    pub name: String,
    /// Bypass the refusal when CWD is the user's home directory or the
    /// filesystem root. Required only for genuinely unusual project roots
    /// (e.g. binding `/` for a system-management workflow).
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, clap::Args)]
pub struct WorkspaceInitArgs {
    /// Workspace root. Defaults to the current directory. Must already
    /// exist — init does NOT create the parent directory.
    pub path: Option<PathBuf>,
    /// Seed the new workspace's `[catalogs]` from the global config.
    /// Enablement state is NEVER copied — enablement lives in the
    /// index DB, not the config.
    #[arg(long = "inherit-global")]
    pub inherit_global: bool,
    /// Replace a pre-existing `.tome/` (rename aside, then remove).
    /// Without `--force`, init refuses on a pre-existing marker.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, clap::Args)]
pub struct McpArgs {
    // No tool-specific flags. `--workspace <name>` comes from
    // `GlobalScopeArgs` on the top-level `Cli`. Empty struct keeps the
    // clap-derive pattern consistent with other commands.
}

#[derive(Debug, clap::Args)]
pub struct StatusArgs {
    /// Rehash each installed model's primary file against its pinned
    /// SHA-256. Slower (several seconds for the reranker), but catches
    /// silent on-disk corruption.
    #[arg(long)]
    pub verify: bool,
}

#[derive(Debug, clap::Args)]
pub struct ReindexArgs {
    /// Scope. Omit to reindex every enabled plugin across every catalog;
    /// pass `<catalog>` to scope to one catalog; pass `<catalog>/<plugin>`
    /// to scope to one plugin.
    pub scope: Option<String>,
    /// Re-embed every in-scope skill regardless of `content_hash`. Used for
    /// embedder upgrades (FR-016 recovery path) and integrity recovery.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Subcommand)]
pub enum ModelsCommand {
    /// Download every registered model that is missing. `--force` re-downloads
    /// even when the on-disk manifest already records a complete install.
    Download(ModelsDownloadArgs),
    /// List every registered model with its on-disk state. `--verify` rehashes
    /// each installed model against its pinned SHA-256.
    List(ModelsListArgs),
    /// Remove an installed model directory and its manifest.
    Remove(ModelsRemoveArgs),
}

#[derive(Debug, clap::Args)]
pub struct ModelsDownloadArgs {
    /// Re-download even when the on-disk manifest records a complete install.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, clap::Args)]
pub struct ModelsListArgs {
    /// Rehash each installed file's contents against its pinned SHA-256.
    /// Slower (several seconds for the reranker) but catches silent
    /// on-disk corruption.
    #[arg(long)]
    pub verify: bool,
}

#[derive(Debug, clap::Args)]
pub struct ModelsRemoveArgs {
    /// The registered model name (e.g. `bge-small-en-v1.5`).
    pub name: String,
    /// Skip the confirmation prompt. Required when stdin is not a TTY.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Subcommand)]
pub enum CatalogCommand {
    /// Register a remote catalog.
    Add(CatalogAddArgs),
    /// Remove a registered catalog.
    Remove(CatalogRemoveArgs),
    /// List registered catalogs.
    List(CatalogListArgs),
    /// Refresh one or every registered catalog.
    Update(CatalogUpdateArgs),
    /// Show the manifest and registration metadata for a catalog.
    Show(CatalogShowArgs),
}

#[derive(Debug, clap::Args)]
pub struct CatalogAddArgs {
    /// The catalog source: an owner/repo shorthand, a Git URL, or a local
    /// path (interpreted as `file://`).
    pub source: String,
    /// Override the display name (defaults to the manifest's `name`).
    #[arg(long)]
    pub name: Option<String>,
    /// Branch, tag, or SHA to track. Defaults to `main`.
    #[arg(long = "ref")]
    pub ref_: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct CatalogRemoveArgs {
    /// The catalog display name to remove.
    pub name: String,
    /// Skip the confirmation prompt. Required when stdin is not a TTY.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, clap::Args)]
pub struct CatalogListArgs {
    // No flags yet — `--json` is global.
}

#[derive(Debug, clap::Args)]
pub struct CatalogUpdateArgs {
    /// The catalog to refresh. Omit to refresh every registered catalog.
    pub name: Option<String>,
    /// Reserved for future symmetry with the other commands. Currently no-op.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, clap::Args)]
pub struct CatalogShowArgs {
    /// The catalog display name to inspect.
    pub name: String,
}

/// Wraps the `plugin` subcommand so the `command` field can be `None` —
/// allowing bare `tome plugin` to drop into the interactive flow.
#[derive(Debug, clap::Args)]
pub struct PluginArgs {
    #[command(subcommand)]
    pub command: Option<PluginCommand>,
}

#[derive(Debug, Subcommand)]
pub enum PluginCommand {
    /// Enable a plugin: index its skills and start surfacing them in queries.
    Enable(PluginEnableArgs),
    /// Disable a plugin: hide its skills from queries while retaining the
    /// embeddings on disk so re-enable is cheap.
    Disable(PluginDisableArgs),
    /// List plugins discoverable across every registered catalog.
    List(PluginListArgs),
    /// Show one plugin's metadata, component counts, and index status.
    Show(PluginShowArgs),
}

#[derive(Debug, clap::Args)]
pub struct PluginEnableArgs {
    /// The plugin to enable, as `<catalog>/<plugin>`.
    pub id: String,
    /// Skip the model-download confirmation prompt. Required to enable a
    /// plugin from a non-interactive context (e.g. CI) when models are
    /// not yet installed.
    #[arg(long)]
    pub yes: bool,
}

#[derive(Debug, clap::Args)]
pub struct PluginDisableArgs {
    /// The plugin to disable, as `<catalog>/<plugin>`.
    pub id: String,
    /// Skip the confirmation prompt. Required to disable a plugin from a
    /// non-interactive context (e.g. CI).
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, clap::Args)]
pub struct PluginListArgs {
    /// Restrict the listing to one catalog.
    #[arg(long)]
    pub catalog: Option<String>,
    /// Hide disabled and unindexable plugins.
    #[arg(long = "enabled-only")]
    pub enabled_only: bool,
}

#[derive(Debug, clap::Args)]
pub struct PluginShowArgs {
    /// The plugin to inspect, as `<catalog>/<plugin>`.
    pub id: String,
}

#[derive(Debug, clap::Args)]
pub struct QueryArgs {
    /// The query text to search for. Embedded as-is — no name/description
    /// composition is applied (cf. FR-014, query.md step 3).
    pub text: String,

    /// Cap on returned results (post-rerank when reranking).
    #[arg(long = "top-k", default_value_t = 10)]
    pub top_k: u32,

    /// Restrict the search to a single catalog.
    #[arg(long)]
    pub catalog: Option<String>,

    /// Restrict the search to a single plugin (across all enabled catalogs
    /// unless `--catalog` is also set).
    #[arg(long)]
    pub plugin: Option<String>,

    /// Skip the reranker stage; scores are cosine similarity.
    #[arg(long = "no-rerank")]
    pub no_rerank: bool,

    /// Apply the score threshold and exit non-zero on empty result.
    #[arg(long)]
    pub strict: bool,

    /// Minimum score to retain a result (only enforced with `--strict`).
    /// Default is 0.0 with the reranker on, 0.5 with `--no-rerank`.
    #[arg(long = "min-score")]
    pub min_score: Option<f32>,
}

impl Cli {
    pub fn verbosity(&self) -> crate::logging::Verbosity {
        crate::logging::Verbosity::from_count(self.verbose)
    }

    pub fn mode(&self) -> crate::output::Mode {
        crate::output::Mode::from_flag(self.json)
    }
}
