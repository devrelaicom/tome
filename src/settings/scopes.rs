//! Canonical scope-loaders for the layered settings walk.
//!
//! Phase 6 / US4 (R4-2): the project-marker / workspace-settings /
//! global-settings loaders were duplicated verbatim across three sites —
//! `commands::harness::list`, `harness::sync`, and the MCP server's
//! `resolve_expose_personas`. Each copy carried the same
//! `NotFound → fall-through` and `parse-error → WorkspaceMalformed` arms
//! and the same reason strings, a textbook drift hazard. They live here
//! once, `pub(crate)`, so every consumer of the (project, workspace,
//! global) settings triple resolves it through one source of truth.
//!
//! The error mapping mirrors the prior copies exactly:
//!
//! - **Project marker**: routed through
//!   [`crate::settings::parser::read_project_marker`], whose canonical
//!   classification splits IO (`TomeError::Io`, exit 7) from parse
//!   (`WorkspaceMalformed`, exit 70). An absent marker collapses to
//!   `Ok(None)` (caller-side Option-wrapping convention).
//! - **Workspace settings**: bounded read; `NotFound → Ok(None)`; parse
//!   failure → `WorkspaceMalformed { reason: "parse workspace settings: …" }`.
//! - **Global settings**: bounded read; `NotFound → Ok(GlobalSettings::default())`;
//!   parse failure → `WorkspaceMalformed { reason: "parse global settings: …" }`.

use crate::error::TomeError;
use crate::paths::Paths;
use crate::settings::parser::{parse_workspace, read_project_marker};
use crate::settings::{GlobalSettings, ProjectMarkerConfig, WorkspaceSettings};
use crate::workspace::WorkspaceName;

/// Load the project-marker config from `<project_root>/.tome/config.toml`.
///
/// `Ok(None)` when the marker is absent (`NotFound`); a parse failure
/// surfaces as [`TomeError::WorkspaceMalformed`] (exit 70) and any other
/// IO failure as [`TomeError::Io`] (exit 7) — both via the canonical
/// [`read_project_marker`] classification.
pub(crate) fn load_project_marker(
    project_root: Option<&std::path::Path>,
) -> Result<Option<ProjectMarkerConfig>, TomeError> {
    let Some(project_root) = project_root else {
        return Ok(None);
    };
    let path = Paths::project_marker_config(project_root);
    match read_project_marker(&path) {
        Ok(pm) => Ok(Some(pm)),
        Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Load `<root>/workspaces/<name>/settings.toml`.
///
/// `Ok(None)` when the file is absent; a parse failure surfaces as
/// [`TomeError::WorkspaceMalformed`] (exit 70).
pub(crate) fn load_workspace_settings(
    paths: &Paths,
    workspace_name: &WorkspaceName,
) -> Result<Option<WorkspaceSettings>, TomeError> {
    let path = paths.workspace_settings_file(workspace_name);
    let body = match crate::util::bounded_read_to_string(&path, crate::util::TOME_CONFIG_MAX) {
        Ok(b) => b,
        Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let ws = parse_workspace(&body).map_err(|e| TomeError::WorkspaceMalformed {
        path: path.clone(),
        reason: format!("parse workspace settings: {e}"),
    })?;
    Ok(Some(ws))
}

/// Load the global harness-settings layer from `<root>/config.toml [harness]`.
///
/// The global harness layer now lives in `config.toml` under the `[harness]`
/// section (Task 2). An absent file collapses to [`GlobalSettings::default`];
/// a parse failure maps to [`TomeError::WorkspaceMalformed`] (exit 70) for
/// backward-compatible error surfacing.
pub(crate) fn load_global_settings(paths: &Paths) -> Result<GlobalSettings, TomeError> {
    crate::config::load(paths)
        .map(|cfg| cfg.harness)
        .map_err(|e| TomeError::WorkspaceMalformed {
            path: paths.global_config_file.clone(),
            reason: format!("parse global config: {e}"),
        })
}
