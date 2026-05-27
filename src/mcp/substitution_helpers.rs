//! Shared `SubstitutionContext` construction for MCP tool/prompt
//! handlers.
//!
//! Polish M-4 (Phase 5): both `mcp::prompts::build_get_context` and
//! `mcp::tools::get_skill::build_substitution_context` derived
//! `entry_dir` + walked ancestors for `.claude-plugin/` + called
//! `substitution::current_clock()` + built a `SubstitutionContext`
//! with the same 12 setters. The only divergences were the args /
//! declared_args (get_skill never accepts args) and the
//! `plugin_version` source. This helper consolidates the duplication;
//! both callers reduce to a one-line call.
//!
//! When the substitution-engine contract grows beyond "walk ancestors
//! looking for `.claude-plugin/`" (e.g. a catalog-manifest-driven
//! plugin_root resolver), this is the single seam to update.

use std::path::{Path, PathBuf};

use crate::paths::Paths;
use crate::substitution::{self, ArgumentValues, SubstitutionContext, SubstitutionError};
use crate::workspace::WorkspaceName;

/// Build the `SubstitutionContext` for an MCP-driven entry render.
///
/// Resolves `entry_dir` from `entry_path.parent()` with a defensive
/// fallback to `entry_path` itself; walks ancestors looking for
/// `.claude-plugin/` to identify `plugin_root_dir`, falling back to
/// `entry_dir` when no marker is found (catalogs older than the
/// `.claude-plugin/` convention).
///
/// `args` + `declared_args` are caller-supplied because the two
/// existing callers diverge on those (prompts accepts caller-mapped
/// arguments; get_skill always passes `None` / empty). `plugin_version`
/// is likewise caller-supplied — registries cache it (prompts) or
/// `LookupHit` carries it (get_skill).
///
/// Failure paths surface a `SubstitutionError`; the caller maps to the
/// surface-specific error envelope (TomeError variant or McpError).
#[allow(clippy::too_many_arguments)]
pub fn build_context_for_entry(
    catalog: String,
    plugin: String,
    plugin_version: String,
    entry_name: String,
    entry_path: PathBuf,
    workspace_name: &WorkspaceName,
    paths: Paths,
    args: Option<ArgumentValues>,
    declared_args: Vec<String>,
) -> Result<SubstitutionContext, SubstitutionError> {
    let entry_dir = entry_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| entry_path.clone());

    let plugin_root_dir = entry_dir
        .ancestors()
        .find(|p| p.join(".claude-plugin").is_dir())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| entry_dir.clone());

    let clock = substitution::current_clock();

    SubstitutionContext::builder()
        .catalog_name(catalog)
        .plugin_name(plugin)
        .plugin_version(plugin_version)
        .entry_name(entry_name)
        .entry_path(entry_path)
        .entry_dir(entry_dir)
        .plugin_root_dir(plugin_root_dir)
        .workspace_name(workspace_name.as_str().to_owned())
        .clock(clock)
        .args(args)
        .declared_args(declared_args)
        .paths(paths)
        .build()
}
