//! Lazy creation of plugin + workspace data directories.
//!
//! Resolved on first reference within a single substitution pass per
//! FR-024. `create_dir_all` is kernel-atomic and idempotent under
//! concurrent retrievals (NFR-012); failure surfaces
//! `SubstitutionError::{PluginDataDirCreationFailed,
//! WorkspaceDataDirCreationFailed}` which the consumer boundary maps to
//! exit 9 / 25.
//!
//! Layout (per research §R-9, anchored under `<home>/.tome/`):
//!
//! - Plugin data:    `<home>/.tome/plugin-data/<catalog>/<plugin>/`
//! - Workspace data: `<home>/.tome/workspaces/<workspace>/plugin-data/<catalog>/<plugin>/`
//!
//! Path components used in directory construction are sanitised per
//! FR-024 via [`sanitise_path_component`]. The `PLUGIN_NAME` /
//! `CATALOG_NAME` built-ins return the unsanitised value (sanitisation
//! is path-only).

use std::path::PathBuf;
use std::sync::PoisonError;

use crate::paths::Paths;
use crate::workspace::WorkspaceName;

use super::SubstitutionError;

/// Replace any character not in `[A-Za-z0-9._-]` with `_` per FR-024.
///
/// Applied ONLY when constructing data-directory paths. Callers reading
/// catalog or plugin names for display (`${TOME_CATALOG_NAME}` /
/// `${TOME_PLUGIN_NAME}`) MUST NOT sanitise.
pub(super) fn sanitise_path_component(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Compute and lazily create the plugin data dir.
///
/// Consults [`super::PLUGIN_DATA_DIR_OVERRIDE`] first; when the slot is
/// `Some(path)` (set by tests via `PluginDataDirGuard`), returns that
/// path directly without touching the filesystem. Otherwise computes
/// `paths.plugin_data_dir_for(sanitise(catalog), sanitise(plugin))`,
/// `create_dir_all`s it, and returns the absolute path.
pub(super) fn ensure_plugin_data(
    paths: &Paths,
    catalog: &str,
    plugin: &str,
) -> Result<PathBuf, SubstitutionError> {
    if let Some(slot) = super::PLUGIN_DATA_DIR_OVERRIDE.get() {
        let guard = slot.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(p) = guard.clone() {
            return Ok(p);
        }
    }
    let cs = sanitise_path_component(catalog);
    let ps = sanitise_path_component(plugin);
    let dir = paths.plugin_data_dir_for(&cs, &ps);
    std::fs::create_dir_all(&dir).map_err(|source| {
        SubstitutionError::PluginDataDirCreationFailed {
            path: dir.clone(),
            source,
        }
    })?;
    Ok(dir)
}

/// Compute and lazily create the workspace data dir.
///
/// Mirrors [`ensure_plugin_data`] but anchors under the per-workspace
/// tree. Consults [`super::WORKSPACE_DATA_DIR_OVERRIDE`] first.
pub(super) fn ensure_workspace_data(
    paths: &Paths,
    workspace: &WorkspaceName,
    catalog: &str,
    plugin: &str,
) -> Result<PathBuf, SubstitutionError> {
    if let Some(slot) = super::WORKSPACE_DATA_DIR_OVERRIDE.get() {
        let guard = slot.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(p) = guard.clone() {
            return Ok(p);
        }
    }
    let cs = sanitise_path_component(catalog);
    let ps = sanitise_path_component(plugin);
    let dir = paths.workspace_data_dir_for(workspace, &cs, &ps);
    std::fs::create_dir_all(&dir).map_err(|source| {
        SubstitutionError::WorkspaceDataDirCreationFailed {
            path: dir.clone(),
            source,
        }
    })?;
    Ok(dir)
}
