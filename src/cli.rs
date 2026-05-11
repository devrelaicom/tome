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

impl Cli {
    pub fn verbosity(&self) -> crate::logging::Verbosity {
        crate::logging::Verbosity::from_count(self.verbose)
    }

    pub fn mode(&self) -> crate::output::Mode {
        crate::output::Mode::from_flag(self.json)
    }
}
