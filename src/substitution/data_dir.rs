//! Lazy creation of plugin + workspace data directories.
//!
//! Resolved on first reference within a single substitution pass; the
//! actual `create_dir_all` is wired by US2 per FR-024. F3 stub just
//! computes the path and returns it without touching the filesystem.
//!
//! Layout (per research §R-9, anchored under `<home>/.tome/`):
//!
//! - Plugin data:    `<home>/.tome/plugin-data/<catalog>/<plugin>/`
//! - Workspace data: `<home>/.tome/workspaces/<workspace>/plugin-data/<catalog>/<plugin>/`

use std::path::{Path, PathBuf};

use super::SubstitutionError;

/// Compute and (in US2) lazily create the plugin data dir.
///
/// F3 stub: computes the path but does NOT create it.
#[allow(dead_code)]
pub(super) fn ensure_plugin_data(
    home_root: &Path,
    catalog: &str,
    plugin: &str,
) -> Result<PathBuf, SubstitutionError> {
    Ok(home_root.join("plugin-data").join(catalog).join(plugin))
}

/// Compute and (in US2) lazily create the workspace data dir.
///
/// F3 stub: computes the path but does NOT create it.
#[allow(dead_code)]
pub(super) fn ensure_workspace_data(
    home_root: &Path,
    workspace: &str,
    catalog: &str,
    plugin: &str,
) -> Result<PathBuf, SubstitutionError> {
    Ok(home_root
        .join("workspaces")
        .join(workspace)
        .join("plugin-data")
        .join(catalog)
        .join(plugin))
}
