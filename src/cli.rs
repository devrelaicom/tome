//! `clap` derive definitions. Globals (`--json`, `-v`/`-vv`) live on the
//! top-level `Cli`; `--force` is per-subcommand but keeps the same name
//! everywhere (FR-021). `--help` and `--version` are auto-supplied by clap
//! (FR-021a).

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "tome", version, about, long_about = None)]
pub struct Cli {
    /// Emit machine-readable JSON on stdout instead of human text.
    #[arg(long, global = true)]
    pub json: bool,

    /// Increase log verbosity. `-v` = info, `-vv` = debug. Env: TOME_LOG.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Manage registered catalogs.
    #[command(subcommand)]
    Catalog(CatalogCommand),
    /// Manage plugins from registered catalogs.
    #[command(subcommand)]
    Plugin(PluginCommand),
    /// Search enabled skills across every catalog.
    Query(QueryArgs),
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

#[derive(Debug, Subcommand)]
pub enum PluginCommand {
    /// Enable a plugin: index its skills and start surfacing them in queries.
    Enable(PluginEnableArgs),
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
